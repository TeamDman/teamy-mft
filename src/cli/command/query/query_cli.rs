use crate::search_index::format::SearchIndexPathRow;
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
use std::sync::Arc;
use teamy_windows::storage::DriveLetterPattern;
use tracing::debug;
use tracing::info;
use tracing::info_span;
use tracing::instrument;

#[derive(Facet, PartialEq, Debug, Arbitrary, Default)]
#[facet(rename_all = "kebab-case")]
pub struct QueryArgs {
    /// Substring to search for (case-insensitive) in resolved paths (first positional)
    #[facet(args::positional)]
    pub query: String,
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

#[derive(Debug, Clone)]
struct IndexedPathRow {
    path: String,
    has_deleted_entries: bool,
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

fn load_and_queue_drive_search_index(
    drive_letter: char,
    sync_dir: &Path,
    injector: &nucleo::Injector<IndexedPathRow>,
    include_deleted: bool,
    only_deleted: bool,
) -> eyre::Result<usize> {
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

    let rows: Vec<SearchIndexPathRow> = {
        let _span = info_span!("decode_search_index_rows").entered();
        mapped.rows().wrap_err_with(|| {
            format!(
                "Failed parsing search index rows for drive {} from {}",
                drive_letter,
                index_path.display()
            )
        })?
    };

    let items: Vec<IndexedPathRow> = {
        let _span = info_span!("filter_and_queue_index_rows", source_rows = rows.len()).entered();
        rows.into_iter()
            .filter(|row| {
                should_include_indexed_row(include_deleted, only_deleted, row.has_deleted_entries)
            })
            .map(|row| IndexedPathRow {
                path: row.path,
                has_deleted_entries: row.has_deleted_entries,
            })
            .collect()
    };

    let queued_rows = items.len();
    for item in items {
        injector.push(item, |item, cols| {
            cols[0] = item.path.clone().into();
        });
    }

    Ok(queued_rows)
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
        let mut nucleo = {
            let _span = info_span!("create_indexed_nucleo_matcher").entered();
            nucleo::Nucleo::<IndexedPathRow>::new(nucleo::Config::DEFAULT, Arc::new(|| {}), None, 1)
        };

        {
            let _span = info_span!("configure_indexed_nucleo_pattern", query = %self.query).entered();
            nucleo.pattern.reparse(
                0,
                &self.query,
                nucleo::pattern::CaseMatching::Smart,
                nucleo::pattern::Normalization::Smart,
                false,
            );
        }

        let mut loaded_rows = 0usize;
        {
            let _span = info_span!("load_search_indexes", drives = mft_files.len()).entered();
            let injector = nucleo.injector();
            let include_deleted = self.include_deleted;
            let only_deleted = self.only_deleted;
            let load_results: Vec<eyre::Result<usize>> = mft_files
                .into_par_iter()
                .map(|(drive_letter, _)| {
                    let injector = injector.clone();
                    load_and_queue_drive_search_index(
                        drive_letter,
                        sync_dir,
                        &injector,
                        include_deleted,
                        only_deleted,
                    )
                })
                .collect();

            for result in load_results {
                loaded_rows += result?;
            }
        }

        loop {
            let status = nucleo.tick(100);
            if !status.running {
                break;
            }
        }

        let snapshot = nucleo.snapshot();
        info!(
            loaded_rows = loaded_rows,
            matched = snapshot.matched_item_count(),
            total = snapshot.item_count(),
            "Indexed query completed"
        );

        let stdout_is_terminal = std::io::stdout().is_terminal();
        let colorize = stdout_is_terminal && (self.include_deleted || self.only_deleted);
        let results: Vec<IndexedPathRow> = snapshot
            .matched_items(..)
            .take(self.limit)
            .map(|item| item.data.clone())
            .collect();

        if self.should_use_columns(stdout_is_terminal) {
            print_results_columns(&results, colorize);
        } else {
            print_results_lines(&results, colorize);
        }

        std::process::exit(0);
    }

    /// Query indexed paths from `.mft_search_index` files.
    ///
    /// # Errors
    ///
    /// Returns an error if the query is empty, sync directory cannot be retrieved,
    /// drive letters cannot be resolved, or if reading/parsing index files fails.
    #[instrument(level = "info", skip_all, fields(query = %self.query, limit = self.limit, include_deleted = self.include_deleted, only_deleted = self.only_deleted, density = ?self.density))]
    pub fn invoke(self) -> eyre::Result<()> {
        debug!("Running query with args: {:?}", self);
        if self.query.trim().is_empty() {
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
