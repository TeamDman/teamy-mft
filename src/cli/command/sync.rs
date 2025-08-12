use crate::drive_letter_pattern::DriveLetterPattern;
use crate::mft_dump::dump_mft_to_file;
use crate::mft_dump::enable_backup_privileges;
use crate::sync_dir::try_get_sync_dir;
use crate::windows::win_elevation::ensure_elevated;
use arbitrary::Arbitrary;
use clap::Args;
use eyre::Context;
use eyre::bail;
use eyre::eyre;
use itertools::Itertools;
use std::fs::create_dir_all;
use tokio::runtime::Builder;
use tokio::task::JoinSet;
use tracing::error;
use tracing::info;

#[derive(Args, Arbitrary, PartialEq, Debug, Default)]
pub struct SyncArgs {
    /// Drive letter pattern to match drives to sync (e.g., "*", "C", "CD", "C,D")
    #[clap(default_value_t = DriveLetterPattern::default())]
    pub drive_pattern: DriveLetterPattern,

    /// Overwrite existing cached MFT files
    #[clap(long, default_value_t = Default::default())]
    pub overwrite_existing: ExistingOutputBehaviour,
}

#[derive(Default, Arbitrary, clap::ValueEnum, Clone, Debug, Eq, PartialEq, strum::Display)]
#[strum(serialize_all = "kebab-case")]
pub enum ExistingOutputBehaviour {
    /// Skip existing files
    Skip,
    /// Overwrite existing files
    #[default]
    Overwrite,
    /// Abort the operation if any existing files are found
    Abort,
}

impl SyncArgs {
    pub fn invoke(self) -> eyre::Result<()> {
        // Ensure we have a sync directory before elevating
        let sync_dir = try_get_sync_dir()?;

        // Elevate if necessary
        ensure_elevated()?;

        // Create the sync directory if it doesn't exist
        info!("Syncing to directory: {}", sync_dir.display());
        create_dir_all(&sync_dir)?;

        // Identify the drives to sync based on the provided pattern
        let drives = self
            .drive_pattern
            .resolve()?
            .into_iter()
            .filter_map(|drive_letter| {
                let drive_output_path = sync_dir.join(format!("{drive_letter}.mft"));
                match (drive_output_path.exists(), &self.overwrite_existing) {
                    (false, _) => Some(Ok((drive_letter, drive_output_path))),
                    (true, ExistingOutputBehaviour::Skip) => None,
                    (true, ExistingOutputBehaviour::Overwrite) => {
                        Some(Ok((drive_letter, drive_output_path)))
                    }
                    (true, ExistingOutputBehaviour::Abort) => Some(Err(eyre!(
                        "Aborting sync: {} already exists",
                        drive_output_path.display()
                    ))),
                }
            })
            .collect::<eyre::Result<Vec<_>>>()?;

        // If no drives matched the pattern, we can't proceed
        if drives.is_empty() {
            bail!("No drives matched the pattern: {}", self.drive_pattern);
        }

        // Enable backup privileges to access system files like $MFT
        enable_backup_privileges().wrap_err("Failed to enable backup privileges")?;

        // Perform the MFT dumping in parallel
        info!(
            "Found {} drives to sync: {}",
            drives.len(),
            drives
                .iter()
                .map(|(drive_letter, _)| drive_letter)
                .join(", ")
        );
        let runtime = Builder::new_multi_thread().enable_all().build()?;
        runtime.block_on(async move {
            let mut work = JoinSet::new();
            for (drive_letter, drive_output_path) in drives {
                work.spawn(async move {
                    info!(
                        "Dumping MFT for drive {} to {}",
                        drive_letter,
                        drive_output_path.display()
                    );
                    // Actual dumping logic would go here
                    dump_mft_to_file(drive_output_path, drive_letter)?;
                    eyre::Ok(())
                });
            }

            info!("Waiting for MFT dumps to complete...");
            while let Some(result) = work.join_next().await {
                if let Err(e) = result {
                    error!("MFT dumping failed: {}", e);
                }
            }
            eyre::Ok(())
        })
    }
}

impl crate::cli::to_args::ToArgs for SyncArgs {}
