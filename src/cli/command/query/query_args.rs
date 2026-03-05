use crate::cli::to_args::ToArgs;
use crate::search_index::load::MappedSearchIndex;
use crate::search_index::format::SearchIndexPathRow;
use crate::sync_dir::try_get_sync_dir;
use arbitrary::Arbitrary;
use clap::Args;
use color_eyre::owo_colors::OwoColorize;
use eyre::Context;
use std::ffi::OsString;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::Arc;
use teamy_windows::storage::DriveLetterPattern;
use tracing::debug;
use tracing::info;
use tracing::info_span;
use tracing::instrument;

#[derive(Args, Arbitrary, PartialEq, Debug, Default)]
pub struct QueryArgs {
    /// Substring to search for (case-insensitive) in resolved paths (first positional)
    pub query: String,
    /// Drive letter pattern to match drives whose cached MFTs will be queried (e.g., "*", "C", "CD", "C,D")
    #[clap(long, default_value_t = DriveLetterPattern::default())]
    pub drive_pattern: DriveLetterPattern,
    /// Maximum number of results to show
    #[clap(long, default_value_t = 100usize)]
    pub limit: usize,
    /// Include paths that contain one or more deleted MFT entries
    #[clap(long)]
    pub include_deleted: bool,
    /// Show only paths that contain one or more deleted MFT entries
    #[clap(long)]
    pub only_deleted: bool,
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

impl QueryArgs {
    fn invoke_indexed(self, mft_files: Vec<(char, PathBuf)>, sync_dir: PathBuf) -> eyre::Result<()> {
        let mut nucleo = {
            let _span = info_span!("create_indexed_nucleo_matcher").entered();
            nucleo::Nucleo::<IndexedPathRow>::new(
                nucleo::Config::DEFAULT,
                Arc::new(|| {}),
                None,
                1,
            )
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
            for (drive_letter, _) in mft_files {
                let index_path = sync_dir.join(format!("{drive_letter}.mft_search_index"));
                if !index_path.is_file() {
                    eyre::bail!(
                        "Fast query requires {}. Run `teamy-mft sync index --drive-pattern {}` first.",
                        index_path.display(),
                        drive_letter
                    );
                }

                let mapped = MappedSearchIndex::open(&index_path).wrap_err_with(|| {
                    format!(
                        "Failed loading search index for drive {} from {}",
                        drive_letter,
                        index_path.display()
                    )
                })?;

                let rows: Vec<SearchIndexPathRow> = mapped.rows().wrap_err_with(|| {
                    format!(
                        "Failed parsing search index rows for drive {} from {}",
                        drive_letter,
                        index_path.display()
                    )
                })?;

                for row in rows {
                    if self.only_deleted && !row.has_deleted_entries {
                        continue;
                    }
                    if !self.only_deleted && !self.include_deleted && row.has_deleted_entries {
                        continue;
                    }

                    let item = IndexedPathRow {
                        path: row.path,
                        has_deleted_entries: row.has_deleted_entries,
                    };
                    injector.push(item, |x, cols| {
                        cols[0] = x.path.clone().into();
                    });
                    loaded_rows += 1;
                }
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

        for (i, item) in snapshot.matched_items(..).enumerate() {
            if i >= self.limit {
                break;
            }
            println!(
                "{}",
                render_indexed_path(
                    item.data,
                    std::io::stdout().is_terminal() && (self.include_deleted || self.only_deleted)
                )
            );
        }

        std::process::exit(0);
    }

    /// Query indexed paths from `.mft_search_index` files.
    ///
    /// # Errors
    ///
    /// Returns an error if the query is empty, sync directory cannot be retrieved,
    /// drive letters cannot be resolved, or if reading/parsing index files fails.
    #[instrument(level = "info", skip_all, fields(query = %self.query, limit = self.limit, include_deleted = self.include_deleted, only_deleted = self.only_deleted))]
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
            self.drive_pattern
                .into_drive_letters()?
                .into_iter()
                .map(|d| (d, sync_dir.join(format!("{d}.mft"))))
                .filter(|(_, p)| p.is_file())
                .collect()
        };
        self.invoke_indexed(mft_files, sync_dir)
    }
}

impl ToArgs for QueryArgs {
    fn to_args(&self) -> Vec<OsString> {
        let mut args = Vec::new();
        // positional query first
        args.push(self.query.clone().into());
        if self.drive_pattern != DriveLetterPattern::default() {
            args.push("--drive-pattern".into());
            args.push(self.drive_pattern.as_ref().into());
        }
        if self.limit != 100 {
            args.push("--limit".into());
            args.push(self.limit.to_string().into());
        }
        if self.include_deleted {
            args.push("--include-deleted".into());
        }
        if self.only_deleted {
            args.push("--only-deleted".into());
        }
        args
    }
}
