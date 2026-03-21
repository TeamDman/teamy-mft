use crate::sync_dir::try_get_sync_dir;
use eyre::bail;
use std::collections::BTreeMap;
use std::fs::create_dir_all;
use std::path::PathBuf;
use teamy_windows::storage::DriveLetterPattern;
use tracing::info;

#[derive(Debug)]
pub struct DriveSyncInfo {
    pub drive_letter: char,
    pub mft_output_path: PathBuf,
    pub index_output_path: PathBuf,
}

pub fn resolve_drive_infos(
    drive_letter_pattern: &DriveLetterPattern,
) -> eyre::Result<BTreeMap<char, DriveSyncInfo>> {
    let sync_dir = try_get_sync_dir()?;

    info!("Syncing in directory: {}", sync_dir.display());
    create_dir_all(&sync_dir)?;

    let drives = drive_letter_pattern
        .into_drive_letters()?
        .into_iter()
        .map(|drive_letter| (drive_letter, sync_dir.join(format!("{drive_letter}.mft"))))
        .collect::<Vec<_>>();

    let mut drive_infos = BTreeMap::default();
    for (drive_letter, drive_output_path) in drives {
        drive_infos.insert(
            drive_letter,
            DriveSyncInfo {
                drive_letter,
                mft_output_path: drive_output_path,
                index_output_path: sync_dir.join(format!("{drive_letter}.mft_search_index")),
            },
        );
    }

    if drive_infos.is_empty() {
        bail!("No drives matched the pattern: {}", drive_letter_pattern);
    }

    Ok(drive_infos)
}
