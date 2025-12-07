use crate::cli::to_args::ToArgs;
use crate::drive_letter_pattern::DriveLetterPattern;
use crate::mft_process::process_mft_file;
use crate::sync_dir::try_get_sync_dir;
use arbitrary::Arbitrary;
use clap::Args;
use eyre::Context;
use eyre::OptionExt;
use nucleo::Nucleo;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use thousands::Separable;
use tokio::task::JoinSet;
use tracing::debug;
use tracing::info;

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
    pub fn invoke(self) -> eyre::Result<()> {
        debug!("Running query with args: {:?}", self);
        if self.query.trim().is_empty() {
            eyre::bail!("query string required")
        }
        let sync_dir = try_get_sync_dir()?;

        let mft_files: Vec<(char, PathBuf)> = self
            .drive_pattern
            .into_drive_letters()?
            .into_iter()
            .map(|d| (d, sync_dir.join(format!("{d}.mft"))))
            .filter(|(_, p)| p.is_file())
            .collect();

        let mut nucleo = nucleo::Nucleo::<PathBuf>::new(
            nucleo::Config::DEFAULT,
            Arc::new(|| {}), // notify callback
            None,            // default threads
            1,               // single column for matching
        );
        nucleo.pattern.reparse(
            0,
            &self.query,
            nucleo::pattern::CaseMatching::Smart,
            nucleo::pattern::Normalization::Smart,
            false,
        );

        let nucleo = {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            let rtn = runtime
                .block_on(async {
                    let mut join_set: JoinSet<eyre::Result<Option<Nucleo<PathBuf>>>> =
                        JoinSet::new();

                    info!(
                        "Processing MFT files for drives: {}",
                        mft_files
                            .iter()
                            .map(|(d, _)| d.to_string())
                            .collect::<Vec<String>>()
                            .join(", ")
                    );
                    for (drive_letter, mft_path) in mft_files {
                        let drive_letter = drive_letter.to_string();
                        let injector = nucleo.injector();
                        join_set.spawn_blocking(move || {
                            let files = process_mft_file(&drive_letter, &mft_path).wrap_err(
                                format!("Failed to process MFT file for drive {drive_letter}"),
                            )?;
                            let count_of_paths_from_drive = files.total_paths();

                            // Build a drive prefix (e.g., "C:\\") and prepend it to each path, adding to the injector
                            let drive_prefix: PathBuf = PathBuf::from(format!("{drive_letter}:\\"));
                            for file_path in files.0.into_iter().flatten() {
                                let full_path = drive_prefix.join(&file_path);
                                injector.push(full_path, |x, cols| {
                                    cols[0] = x.to_string_lossy().into();
                                });
                            }

                            let count_of_paths_total = injector.injected_items();

                            debug!(
                                drive_letter = &drive_letter,
                                "Added {} paths to be queried against, up to {}",
                                count_of_paths_from_drive.separate_with_commas(),
                                count_of_paths_total.separate_with_commas(),
                            );
                            eyre::Ok(None)
                        });
                    }

                    // Only stop ticking nucleo when all MFT processing tasks are done
                    let remaining = Arc::new(AtomicUsize::new(join_set.len()));
                    let remaining_clone = Arc::clone(&remaining);

                    join_set.spawn_blocking(move || {
                        debug!("Ticking Nucleo...");
                        loop {
                            let status = nucleo.tick(100);
                            if !status.running
                                && remaining_clone.load(std::sync::atomic::Ordering::Relaxed) == 0
                            {
                                break;
                            }
                            debug!("Tick status: {:?}", status);
                        }
                        eyre::Ok(Some(nucleo))
                    });

                    let mut nucleo: Option<Nucleo<PathBuf>> = None;
                    while let Some(res) = join_set.join_next().await {
                        nucleo = nucleo.or(res??);
                        remaining.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                    }
                    eyre::Ok(nucleo)
                })?
                .ok_or_eyre("Failed to get back Nucleo after handing off for parallel ticking")?;

            // Leak the runtime to skip running drop handlers, we will exit very soon.
            Box::leak(Box::new(runtime));
            rtn
        };

        let snapshot = nucleo.snapshot();
        info!(
            "Found {} matching items out of {} total items, showing up to {}",
            snapshot.matched_item_count().separate_with_commas(),
            snapshot.item_count().separate_with_commas(),
            self.limit.separate_with_commas()
        );
        for (i, item) in snapshot.matched_items(..).enumerate() {
            if i >= self.limit {
                break;
            }
            println!("{}", item.data.display());
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
            args.push(self.drive_pattern.as_str().into());
        }
        if self.limit != 100 {
            args.push("--limit".into());
            args.push(self.limit.to_string().into());
        }
        args
    }
}
