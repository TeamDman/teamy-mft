use crate::cli::to_args::ToArgs;
use crate::drive_letter_pattern::DriveLetterPattern;
use crate::mft::mft_physical_read::read_physical_mft;
use crate::ntfs::ntfs_drive_handle::get_volume_disk_extents;
use crate::sync_dir::try_get_sync_dir;
use arbitrary::Arbitrary;
use clap::Args;
use crossbeam_channel::bounded;
use eyre::Context;
use eyre::bail;
use eyre::eyre;
use itertools::Itertools;
use std::ffi::OsString;
use std::fs::create_dir_all;
use std::path::PathBuf;
use std::thread;
use teamy_windows::elevation::enable_backup_privileges;
use teamy_windows::elevation::ensure_elevated;
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

#[derive(Debug)]
struct DriveInfo {
    drive_letter: char,
    output_path: PathBuf,
    disk_number: u32,
    starting_offset: i64,
}

impl SyncArgs {
    /// Sync MFT data from drives.
    ///
    /// # Errors
    ///
    /// Returns an error if the sync directory cannot be retrieved, elevation fails,
    /// or if reading/writing MFT data fails.
    ///
    /// # Panics
    ///
    /// Panics if spawning worker threads fails.
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
            .into_drive_letters()?
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

        let mut drive_infos = Vec::new();
        for (drive_letter, drive_output_path) in drives {
            let extents = get_volume_disk_extents(drive_letter)?;
            let extent = &extents.Extents[0];
            drive_infos.push(DriveInfo {
                drive_letter,
                output_path: drive_output_path,
                disk_number: extent.DiskNumber,
                starting_offset: extent.StartingOffset,
            });
        }
        drive_infos.sort_by_key(|info| (info.disk_number, info.starting_offset));

        // If no drives matched the pattern, we can't proceed
        if drive_infos.is_empty() {
            bail!("No drives matched the pattern: {}", self.drive_pattern);
        }

        // Enable backup privileges to access system files like $MFT
        enable_backup_privileges().wrap_err("Failed to enable backup privileges")?;

        info!(
            "Found {} drives to sync: {}",
            drive_infos.len(),
            drive_infos.iter().map(|info| info.drive_letter).join(", ")
        );

        // ---- IOCP worker-pool flow ----
        let max_workers = drive_infos.len();
        let (tx, rx) = bounded::<(char, PathBuf)>(drive_infos.len());

        let mut handles = Vec::with_capacity(max_workers);
        for worker_id in 0..max_workers {
            let rx = rx.clone();
            let handle = thread::Builder::new()
                .name(format!("mft-iocp-{worker_id}"))
                .spawn(move || {
                    while let Ok((drive_letter, output_path)) = rx.recv() {
                        info!(
                            "Worker {} reading drive {} -> {}",
                            worker_id,
                            drive_letter,
                            output_path.display()
                        );

                        match read_physical_mft(drive_letter) {
                            Ok((logical_segments, physical_read_results)) => {
                                physical_read_results
                                    .write_to_file(&logical_segments, &output_path)
                                    .wrap_err("Failed writing MFT output file")?;
                            }
                            Err(e) => {
                                error!(
                                    "Worker {}: IOCP read failed for {}: {:#}",
                                    worker_id, drive_letter, e
                                );
                            }
                        }
                    }
                    eyre::Ok(())
                })
                .wrap_err("Failed to spawn IOCP worker thread")
                .unwrap();
            handles.push(handle);
        }

        for info in &drive_infos {
            tx.send((info.drive_letter, info.output_path.clone()))
                .wrap_err("Failed to schedule IOCP drive job")?;
        }
        drop(tx);

        for handle in handles {
            handle
                .join()
                .map_err(|e| eyre::eyre!("Failed to join worker: {:?}", e))?
                .wrap_err("Identified failure result from worker")?;
        }

        info!("All MFT dumps completed");

        Ok(())
    }
}

impl ToArgs for SyncArgs {
    fn to_args(&self) -> Vec<OsString> {
        let mut args = Vec::new();
        if self.drive_pattern != DriveLetterPattern::default() {
            args.push(self.drive_pattern.as_str().into());
        }
        if self.overwrite_existing != ExistingOutputBehaviour::default() {
            args.push("--overwrite-existing".into());
            args.push(self.overwrite_existing.to_string().into());
        }
        args
    }
}
