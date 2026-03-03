use crate::cli::to_args::ToArgs;
use crate::mft::path_resolve::ResolvedPath;
use crate::mft_process::process_mft_file;
use crate::sync_dir::try_get_sync_dir;
use arbitrary::Arbitrary;
use clap::Args;
use color_eyre::owo_colors::OwoColorize;
use eyre::Context;
use eyre::OptionExt;
use nucleo::Nucleo;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::Instant;
use teamy_windows::storage::DriveLetterPattern;
use thousands::Separable;
use tokio::task::JoinSet;
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
}

fn render_resolved_path(path: &ResolvedPath, colorize: bool) -> String {
    if !colorize {
        return path.path.display().to_string();
    }

    let mut rendered = String::new();
    rendered.push_str(&path.root_prefix);
    for (index, component) in path.components.iter().enumerate() {
        if !rendered.ends_with('\\')
            && !rendered.ends_with('/')
            && (index > 0 || !path.root_prefix.is_empty())
        {
            rendered.push('\\');
        }
        let is_deleted = path.component_deleted.get(index).copied().unwrap_or(false);
        if is_deleted {
            rendered.push_str(&component.red().to_string());
        } else {
            rendered.push_str(&component.green().to_string());
        }
    }
    rendered
}

impl QueryArgs {
    /// Query MFT files for entries matching the given query.
    ///
    /// # Errors
    ///
    /// Returns an error if the query is empty, sync directory cannot be retrieved,
    /// drive letters cannot be resolved, or if reading/parsing MFT files fails.
    #[allow(
        clippy::too_many_lines,
        reason = "function handles complex query logic with multiple threads"
    )]
    #[instrument(level = "info", skip_all, fields(query = %self.query, limit = self.limit, include_deleted = self.include_deleted))]
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

        let mut nucleo = {
            let _span = info_span!("create_nucleo_matcher").entered();
            nucleo::Nucleo::<ResolvedPath>::new(
                nucleo::Config::DEFAULT,
                Arc::new(|| {}), // notify callback
                None,            // default threads
                1,               // single column for matching
            )
        };

        {
            let _span = info_span!("configure_nucleo_pattern", query = %self.query).entered();
            nucleo.pattern.reparse(
                0,
                &self.query,
                nucleo::pattern::CaseMatching::Smart,
                nucleo::pattern::Normalization::Smart,
                false,
            );
        }

        let nucleo = {
            let _span = info_span!("load_and_match_paths", drives = mft_files.len()).entered();
            let runtime = {
                let _span = info_span!("build_tokio_runtime").entered();
                tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()?
            };
            let rtn = runtime
                .block_on(async {
                    let mut join_set: JoinSet<eyre::Result<Option<Nucleo<ResolvedPath>>>> =
                        JoinSet::new();

                    info!(
                        "Processing MFT files for drives: {}",
                        mft_files
                            .iter()
                            .map(|(d, _)| d.to_string())
                            .collect::<Vec<String>>()
                            .join(", ")
                    );
                    {
                        let _span = info_span!("spawn_drive_tasks", drives = mft_files.len()).entered();
                        for (drive_letter, mft_path) in mft_files {
                            let drive_letter = drive_letter.to_string();
                            let include_deleted = self.include_deleted;
                            let injector = nucleo.injector();
                            debug!(drive_letter = %drive_letter, "Spawning drive worker task");
                            join_set.spawn_blocking(move || {
                                let _span =
                                    info_span!("process_drive", drive_letter = %drive_letter)
                                        .entered();
                                let total_started_at = Instant::now();

                                debug!(
                                    drive_letter = %drive_letter,
                                    include_deleted,
                                    "Starting drive processing"
                                );

                                let mft_load_started_at = Instant::now();
                                let files = {
                                    let _span = info_span!(
                                        "load_and_parse_mft",
                                        drive_letter = %drive_letter
                                    )
                                    .entered();
                                    process_mft_file(&drive_letter, &mft_path).wrap_err(format!(
                                        "Failed to process MFT file for drive {drive_letter}",
                                    ))?
                                };
                                let mft_load_elapsed_ms = mft_load_started_at.elapsed().as_millis();
                                let count_of_paths_from_drive = {
                                    let _span = info_span!(
                                        "count_paths_from_drive",
                                        drive_letter = %drive_letter
                                    )
                                    .entered();
                                    files.total_paths()
                                };

                                debug!(
                                    drive_letter = %drive_letter,
                                    path_count = count_of_paths_from_drive,
                                    elapsed_ms = mft_load_elapsed_ms,
                                    "Loaded and parsed MFT file"
                                );

                                let inject_started_at = Instant::now();
                                let mut scanned_path_count = 0usize;
                                let mut deleted_filtered_count = 0usize;
                                let mut injected_count = 0usize;

                                {
                                    let _span = info_span!(
                                        "filter_and_inject_paths",
                                        drive_letter = %drive_letter,
                                        include_deleted = include_deleted
                                    )
                                    .entered();
                                    for file_path in files.0.into_iter().flatten() {
                                        scanned_path_count += 1;
                                        if !include_deleted && file_path.has_deleted_entries() {
                                            deleted_filtered_count += 1;
                                        } else {
                                            injector.push(file_path, |x, cols| {
                                                cols[0] = x.path.to_string_lossy().into();
                                            });
                                            injected_count += 1;
                                        }
                                    }
                                }

                                let inject_elapsed_ms = inject_started_at.elapsed().as_millis();

                                let count_of_paths_total = {
                                    let _span =
                                        info_span!("query_injected_items_count").entered();
                                    injector.injected_items()
                                };

                                debug!(
                                    drive_letter = %drive_letter,
                                    scanned_path_count = scanned_path_count,
                                    deleted_filtered_count = deleted_filtered_count,
                                    injected_count = injected_count,
                                    count_of_paths_total = count_of_paths_total,
                                    inject_elapsed_ms = inject_elapsed_ms,
                                    total_elapsed_ms = total_started_at.elapsed().as_millis(),
                                    "Finished drive processing"
                                );

                                debug!(
                                    drive_letter = %drive_letter,
                                    "Added {} paths from this drive (scanned {}, filtered deleted {}), up to {} total in injector",
                                    injected_count.separate_with_commas(),
                                    scanned_path_count.separate_with_commas(),
                                    deleted_filtered_count.separate_with_commas(),
                                    count_of_paths_total.separate_with_commas(),
                                );
                                eyre::Ok(None)
                            });
                        }
                    }

                    // Only stop ticking nucleo when all MFT processing tasks are done
                    let remaining = Arc::new(AtomicUsize::new(join_set.len()));
                    let remaining_clone = Arc::clone(&remaining);

                    join_set.spawn_blocking(move || {
                        let _span = info_span!("nucleo_tick_loop").entered();
                        debug!("Ticking Nucleo...");
                        let tick_started_at = Instant::now();
                        loop {
                            let status = nucleo.tick(100);
                            if !status.running
                                && remaining_clone.load(std::sync::atomic::Ordering::Relaxed) == 0
                            {
                                break;
                            }
                        }
                        debug!(
                            elapsed_ms = tick_started_at.elapsed().as_millis(),
                            "Tick loop complete"
                        );
                        eyre::Ok(Some(nucleo))
                    });

                    let mut nucleo: Option<Nucleo<ResolvedPath>> = None;
                    {
                        let _span = info_span!("await_worker_results").entered();
                        while let Some(res) = join_set.join_next().await {
                            let task_result = res??;
                            match task_result {
                                Some(returned_nucleo) => {
                                    debug!("Received Nucleo instance from tick loop task");
                                    nucleo = Some(returned_nucleo);
                                }
                                None => {
                                    let previous_remaining =
                                        remaining.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                                    debug!(
                                        remaining_before = previous_remaining,
                                        remaining_after = previous_remaining.saturating_sub(1),
                                        "Drive worker completed"
                                    );
                                }
                            }
                        }
                    }
                    eyre::Ok(nucleo)
                })?
                .ok_or_eyre("Failed to get back Nucleo after handing off for parallel ticking")?;

            // Leak the runtime to skip running drop handlers, we will exit very soon.
            Box::leak(Box::new(runtime));
            rtn
        };

        let snapshot = {
            let _span = info_span!("snapshot_results").entered();
            nucleo.snapshot()
        };
        info!(
            "Found {} matching items out of {} total items, showing up to {}",
            snapshot.matched_item_count().separate_with_commas(),
            snapshot.item_count().separate_with_commas(),
            self.limit.separate_with_commas()
        );
        {
            let _span = info_span!("print_results").entered();
            for (i, item) in snapshot.matched_items(..).enumerate() {
                if i >= self.limit {
                    break;
                }
                println!("{}", render_resolved_path(item.data, self.include_deleted));
            }
        }

        // Skip drop handlers for faster exit
        std::process::exit(0);
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
        args
    }
}
