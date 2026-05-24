use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};
use std::time::SystemTime;
use teamy_windows::storage::DriveLetterPattern;

/// Show freshness information for cached `.mft` and `.mft_search_index` files.
#[derive(Facet, Arbitrary, PartialEq, Debug, Default)]
#[facet(rename_all = "kebab-case")]
pub struct StatusArgs {
    /// Drive letter pattern to inspect (e.g., `*`, `C`, `CD`, `C,D`).
    #[facet(args::named, default)]
    pub drive_letter_pattern: DriveLetterPattern,
}

impl StatusArgs {
    /// # Errors
    ///
    /// Returns an error if the sync directory is unset, drive letters cannot be resolved,
    /// or cached file metadata cannot be read.
    pub fn invoke(self) -> eyre::Result<()> {
        if let Some(machine_config) = crate::machine::config::load_machine_config()? {
            let machine_status =
                crate::machine::status::load_machine_status(&self.drive_letter_pattern)?;
            println!("machine-cache-root={}", machine_config.cache_root.display());
            println!("machine-service-name={}", machine_config.service_name);
            println!("machine-service-state={:?}", machine_status.service_state);
            println!("machine-owner-sid={}", machine_config.owner_sid);
            println!(
                "machine-current-user-sid={}",
                machine_status
                    .current_user_sid
                    .unwrap_or_else(|| String::from("unknown"))
            );
            println!("machine-owner-access={}", machine_status.owner_access);
            for drive in &machine_status.drives {
                println!("machine-drive={}", drive.drive_letter);
                println!(
                    "machine-drive-{}-mft-path={}",
                    drive.drive_letter,
                    drive.mft_path.display()
                );
                println!(
                    "machine-drive-{}-mft-modified-at={}",
                    drive.drive_letter,
                    crate::status::format_optional_system_time(drive.mft_modified_at)
                );
                println!(
                    "machine-drive-{}-base-index-path={}",
                    drive.drive_letter,
                    drive.base_index_path.display()
                );
                println!(
                    "machine-drive-{}-base-index-modified-at={}",
                    drive.drive_letter,
                    crate::status::format_optional_system_time(drive.base_index_modified_at)
                );
                println!(
                    "machine-drive-{}-overlay-index-path={}",
                    drive.drive_letter,
                    drive.overlay_index_path.display()
                );
                println!(
                    "machine-drive-{}-overlay-index-modified-at={}",
                    drive.drive_letter,
                    crate::status::format_optional_system_time(drive.overlay_index_modified_at)
                );
                println!(
                    "machine-drive-{}-checkpoint-path={}",
                    drive.drive_letter,
                    drive.checkpoint_path.display()
                );
                println!(
                    "machine-drive-{}-checkpoint-modified-at={}",
                    drive.drive_letter,
                    crate::status::format_optional_system_time(drive.checkpoint_modified_at)
                );
                println!(
                    "machine-drive-{}-journal-id={}",
                    drive.drive_letter,
                    drive
                        .checkpoint
                        .as_ref()
                        .and_then(|checkpoint| checkpoint.journal_id)
                        .map_or_else(|| String::from("none"), |value| value.to_string())
                );
                println!(
                    "machine-drive-{}-snapshot-usn={}",
                    drive.drive_letter,
                    drive
                        .checkpoint
                        .as_ref()
                        .and_then(|checkpoint| checkpoint.snapshot_usn)
                        .map_or_else(|| String::from("none"), |value| value.to_string())
                );
                println!(
                    "machine-drive-{}-last-usn={}",
                    drive.drive_letter,
                    drive
                        .checkpoint
                        .as_ref()
                        .and_then(|checkpoint| checkpoint.last_usn)
                        .map_or_else(|| String::from("none"), |value| value.to_string())
                );
            }
        }

        let Some(_legacy_sync_dir) = crate::sync_dir::get_sync_dir()? else {
            return Ok(());
        };
        let status = crate::status::TeamyMftStatus::load(&self.drive_letter_pattern)?;
        let now = SystemTime::now();

        println!("sync-dir={}", status.sync_dir.display());
        println!("drive-count={}", status.drives.len());
        println!(
            "query-ready-drive-count={}",
            status.query_ready_drive_count()
        );
        println!(
            "oldest-query-ready-at={}",
            crate::status::format_optional_system_time(status.oldest_query_ready_at())
        );
        println!(
            "newest-query-ready-at={}",
            crate::status::format_optional_system_time(status.newest_query_ready_at())
        );
        println!(
            "oldest-query-ready-age={}",
            crate::status::format_optional_duration(status.oldest_query_ready_age(now))
        );
        println!(
            "newest-query-ready-age={}",
            crate::status::format_optional_duration(status.newest_query_ready_age(now))
        );

        for drive in &status.drives {
            println!("drive={}", drive.drive_letter);
            println!(
                "drive-{}-mft-path={}",
                drive.drive_letter,
                drive.mft_path.display()
            );
            println!(
                "drive-{}-mft-modified-at={}",
                drive.drive_letter,
                crate::status::format_optional_system_time(drive.mft_modified_at)
            );
            println!(
                "drive-{}-index-path={}",
                drive.drive_letter,
                drive.index_path.display()
            );
            println!(
                "drive-{}-index-modified-at={}",
                drive.drive_letter,
                crate::status::format_optional_system_time(drive.index_modified_at)
            );
            println!(
                "drive-{}-query-ready={}",
                drive.drive_letter,
                drive.is_query_ready()
            );
            println!(
                "drive-{}-query-ready-at={}",
                drive.drive_letter,
                crate::status::format_optional_system_time(drive.query_ready_at())
            );
            println!(
                "drive-{}-query-ready-age={}",
                drive.drive_letter,
                crate::status::format_optional_duration(drive.query_ready_age(now))
            );
        }

        Ok(())
    }
}
