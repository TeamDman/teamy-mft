use crate::drive_letter_pattern::DriveLetterPattern;
use crate::mft_check::process_mft_file;
use crate::sync_dir::try_get_sync_dir;
use arbitrary::Arbitrary;
use clap::Args;
use eyre::Context;
use rayon::iter::IntoParallelRefIterator;
use rayon::iter::ParallelIterator;
use thousands::Separable;
use tracing::debug;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
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
    pub fn invoke(self) -> eyre::Result<()> {
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
        let injector = nucleo.injector();

        mft_files
            .par_iter()
            .map(|(drive_letter, mft_path)| {
                let (files, _stats) =
                    process_mft_file(drive_letter.to_string(), &mft_path, 0, true).wrap_err_with(
                        || format!("Failed to process MFT file for drive {}", drive_letter),
                    )?;
                info!("Found {} files", files.total_paths().separate_with_commas());
                files.0.into_iter().flatten().for_each(|file_path| {
                    injector.push(file_path, |x, cols| {
                        cols[0] = x.to_string_lossy().into();
                    });
                });
                eyre::Ok(())
            })
            .for_each(|resp| {
                if let Err(e) = resp {
                    eprintln!("Error processing MFT file: {:?}", e);
                }
            });

        info!("Ticking...");
        loop {
            let status = nucleo.tick(100);
            if !status.running {
                break;
            }
            debug!("Tick status: {:?}", status);
        }


        let snapshot = nucleo.snapshot();
        info!("Found {} matching items", snapshot.matched_item_count());
        for item in snapshot.matched_items(..) {
            println!("{}", item.data.display());
        }

        std::process::exit(0); // exit intentionally to accelerate cleanup of background threads
    }
}

impl crate::cli::to_args::ToArgs for QueryArgs {
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
