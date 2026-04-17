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
