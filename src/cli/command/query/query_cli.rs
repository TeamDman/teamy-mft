use crate::query::IndexedPathRow;
use crate::query::QueryPlan;
use crate::search_index::load::MappedSearchIndex;
use crate::sync_dir::try_get_sync_dir;
use arbitrary::Arbitrary;
use color_eyre::owo_colors::OwoColorize;
use eyre::Context;
use facet::Facet;
use figue::{self as args};
use rayon::prelude::*;
use std::io::IsTerminal;
use std::path::Path;
use std::path::PathBuf;
use teamy_windows::storage::DriveLetterPattern;
use tracing::debug;
use tracing::info;
use tracing::info_span;
use tracing::instrument;

#[derive(Facet, PartialEq, Debug, Arbitrary, Default)]
#[facet(rename_all = "kebab-case")]
pub struct QueryArgs {
    /// Fast query groups. Each positional argument is `OR`ed; whitespace-delimited terms within one argument are `AND`ed.
    #[facet(args::positional, default)]
    pub query: Vec<String>,
    /// Drive letter pattern to match drives whose cached MFTs will be queried (e.g., "*", "C", "CD", "C,D")
    #[facet(args::named, default)]
    pub drive_letter_pattern: DriveLetterPattern,
    /// Maximum number of results to show
    #[facet(args::named, default)]
    pub limit: usize,
    /// Include paths that contain one or more deleted MFT entries
    #[facet(args::named, default)]
    pub include_deleted: bool,
    /// Show only paths that contain one or more deleted MFT entries
    #[facet(args::named, default)]
    pub only_deleted: bool,
    /// Output density mode
    #[facet(args::named, default)]
    pub density: QueryDensity,
}

#[derive(Default, Facet, Arbitrary, Clone, Copy, Debug, Eq, PartialEq, strum::Display)]
#[repr(u8)]
#[strum(serialize_all = "kebab-case")]
#[facet(rename_all = "kebab-case")]
pub enum QueryDensity {
    #[default]
    Auto,
    Lines,
    Columns,
}

#[derive(Debug, Default)]
struct DriveQueryResult {
    loaded_rows: usize,
    matched_rows: Vec<IndexedPathRow>,
}

fn render_indexed_path(row: &IndexedPathRow, colorize: bool) -> String {
    if !colorize {
        return row.path.clone();
    }
    if row.has_deleted_entries {
        row.path.red().to_string()
    } else {
        row.path.green().to_string()
    }
}

fn string_display_width(value: &str) -> usize {
    value.chars().count()
}

fn detect_terminal_columns() -> Option<usize> {
    crossterm::terminal::size()
        .ok()
        .map(|(columns, _)| usize::from(columns))
        .filter(|value| *value > 0)
        .or_else(|| {
            std::env::var("COLUMNS")
                .ok()
                .and_then(|value| value.parse::<usize>().ok())
                .filter(|value| *value > 0)
        })
}

fn print_results_lines(results: &[IndexedPathRow], colorize: bool) {
    for row in results {
        println!("{}", render_indexed_path(row, colorize));
    }
}

fn print_results_columns(results: &[IndexedPathRow], colorize: bool) {
    if results.is_empty() {
        return;
    }

    let gap = 2usize;
    let max_width = results
        .iter()
        .map(|row| string_display_width(&row.path))
        .max()
        .unwrap_or(1)
        .max(1);
    let terminal_columns = detect_terminal_columns().unwrap_or(120usize);

    let column_count = ((terminal_columns + gap) / (max_width + gap)).max(1);
    let row_count = results.len().div_ceil(column_count);

    for row_index in 0..row_count {
        let mut line = String::new();

        for column_index in 0..column_count {
            let index = row_index + column_index * row_count;
            if index >= results.len() {
                continue;
            }

            let row = &results[index];
            line.push_str(&render_indexed_path(row, colorize));

            if column_index + 1 < column_count {
                let pad = (max_width + gap).saturating_sub(string_display_width(&row.path));
                line.push_str(&" ".repeat(pad));
            }
        }

        println!("{line}");
    }
}

fn should_include_indexed_row(
    include_deleted: bool,
    only_deleted: bool,
    has_deleted_entries: bool,
) -> bool {
    if only_deleted {
        return has_deleted_entries;
    }

    include_deleted || !has_deleted_entries
}

fn load_and_query_drive_search_index(
    drive_letter: char,
    sync_dir: &Path,
    query_plan: &QueryPlan,
    include_deleted: bool,
    only_deleted: bool,
) -> eyre::Result<DriveQueryResult> {
    let _span = info_span!(
        "load_drive_search_index",
        drive = %drive_letter,
    )
    .entered();
    let index_path = sync_dir.join(format!("{drive_letter}.mft_search_index"));

    {
        let _span = info_span!(
            "validate_search_index_file",
            path = %index_path.display(),
        )
        .entered();
        if !index_path.is_file() {
            eyre::bail!(
                "Fast query requires {}. Run `teamy-mft sync index --drive-pattern {}` first.",
                index_path.display(),
                drive_letter
            );
        }
    }

    let mapped = {
        let _span = info_span!(
            "map_search_index_file",
            path = %index_path.display(),
        )
        .entered();
        MappedSearchIndex::open(&index_path).wrap_err_with(|| {
            format!(
                "Failed loading search index for drive {} from {}",
                drive_letter,
                index_path.display()
            )
        })?
    };

    let (loaded_rows, matched_rows) = {
        let _span = info_span!("filter_and_match_index_rows").entered();
        let mut loaded_rows = 0usize;
        let mut matched_rows = Vec::new();

        for row in mapped.row_views().wrap_err_with(|| {
            format!(
                "Failed preparing search index rows for drive {} from {}",
                drive_letter,
                index_path.display()
            )
        })? {
            let row = row.wrap_err_with(|| {
                format!(
                    "Failed parsing search index rows for drive {} from {}",
                    drive_letter,
                    index_path.display()
                )
            })?;
            loaded_rows += 1;

            if !should_include_indexed_row(include_deleted, only_deleted, row.has_deleted_entries) {
                continue;
            }
            if !query_plan.matches_preprocessed(row.path, row.normalized_path) {
                continue;
            }

            matched_rows.push(IndexedPathRow {
                path: row.path.to_owned(),
                has_deleted_entries: row.has_deleted_entries,
            });
        }

        (loaded_rows, matched_rows)
    };

    Ok(DriveQueryResult {
        loaded_rows,
        matched_rows,
    })
}

impl QueryArgs {
    fn should_use_columns(&self, stdout_is_terminal: bool) -> bool {
        match self.density {
            QueryDensity::Auto => stdout_is_terminal,
            QueryDensity::Lines => false,
            QueryDensity::Columns => true,
        }
    }

    fn invoke_indexed(self, mft_files: Vec<(char, PathBuf)>, sync_dir: &Path) -> eyre::Result<()> {
        let query_plan = {
            let _span = info_span!("parse_query_rules", query = ?self.query).entered();
            QueryPlan::parse_inputs(&self.query)?
        };

        let mut loaded_rows = 0usize;
        let mut results = Vec::new();
        {
            let _span = info_span!("load_search_indexes", drives = mft_files.len()).entered();
            let include_deleted = self.include_deleted;
            let only_deleted = self.only_deleted;
            let load_results: Vec<eyre::Result<DriveQueryResult>> = mft_files
                .into_par_iter()
                .map(|(drive_letter, _)| {
                    load_and_query_drive_search_index(
                        drive_letter,
                        sync_dir,
                        &query_plan,
                        include_deleted,
                        only_deleted,
                    )
                })
                .collect();

            for result in load_results {
                let result = result?;
                loaded_rows += result.loaded_rows;
                results.extend(result.matched_rows);
            }
        }

        info!(
            loaded_rows = loaded_rows,
            matched = results.len(),
            total = loaded_rows,
            "Indexed query completed"
        );

        let stdout_is_terminal = std::io::stdout().is_terminal();
        let colorize = stdout_is_terminal && (self.include_deleted || self.only_deleted);
        let result_limit = if self.limit == 0 {
            results.len()
        } else {
            self.limit.min(results.len())
        };
        let display_results = &results[..result_limit];

        if self.should_use_columns(stdout_is_terminal) {
            print_results_columns(display_results, colorize);
        } else {
            print_results_lines(display_results, colorize);
        }

        std::process::exit(0);
    }

    /// Query indexed paths from `.mft_search_index` files.
    ///
    /// # Errors
    ///
    /// Returns an error if the query is empty, sync directory cannot be retrieved,
    /// drive letters cannot be resolved, or if reading/parsing index files fails.
    #[instrument(level = "info", skip_all, fields(query = ?self.query, limit = self.limit, include_deleted = self.include_deleted, only_deleted = self.only_deleted, density = ?self.density))]
    pub fn invoke(self) -> eyre::Result<()> {
        debug!("Running query with args: {:?}", self);
        if self.query.iter().all(|query| query.trim().is_empty()) {
            eyre::bail!("query string required")
        }
        let sync_dir = {
            let _span = info_span!("resolve_sync_dir").entered();
            try_get_sync_dir()?
        };

        let mft_files: Vec<(char, PathBuf)> = {
            let _span = info_span!("discover_mft_files").entered();
            self.drive_letter_pattern
                .into_drive_letters()?
                .into_iter()
                .map(|d| (d, sync_dir.join(format!("{d}.mft"))))
                .filter(|(_, p)| p.is_file())
                .collect()
        };
        self.invoke_indexed(mft_files, &sync_dir)
    }
}
