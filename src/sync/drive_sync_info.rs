use crate::machine::config::published_drive_paths;
use crate::windows_utils::storage::DriveLetterPattern;
use eyre::bail;
use std::fs::create_dir_all;
use std::path::Path;
use std::path::PathBuf;
use tracing::info;

#[derive(Debug, Clone)]
pub struct DriveSyncInfo {
    pub drive_letter: char,
    pub mft_output_path: PathBuf,
    pub index_output_path: PathBuf,
    pub overlay_output_path: PathBuf,
    pub checkpoint_output_path: PathBuf,
}

/// # Errors
///
/// Returns an error if the machine cache root cannot be resolved or if no drives match.
pub fn resolve_drive_infos(
    drive_letter_pattern: &DriveLetterPattern,
) -> eyre::Result<Vec<DriveSyncInfo>> {
    let sync_dir = crate::machine::config::load_sync_dir_from_config()?;
    resolve_drive_infos_in_dir(sync_dir.as_path(), drive_letter_pattern)
}

/// # Errors
///
/// Returns an error if the drive pattern cannot be expanded or if no drives match.
pub fn resolve_drive_infos_in_dir(
    sync_dir: &Path,
    drive_letter_pattern: &DriveLetterPattern,
) -> eyre::Result<Vec<DriveSyncInfo>> {
    resolve_drive_infos_in_dir_for_letters(sync_dir, drive_letter_pattern.into_drive_letters()?)
}

/// # Errors
///
/// Returns an error if the sync directory cannot be created or if no drives match.
pub fn resolve_drive_infos_in_dir_for_letters(
    sync_dir: &Path,
    drive_letters: impl IntoIterator<Item = char>,
) -> eyre::Result<Vec<DriveSyncInfo>> {
    info!("Syncing in directory: {}", sync_dir.display());
    create_dir_all(sync_dir)?;

    let mut drive_infos = drive_letters
        .into_iter()
        .map(|drive_letter| {
            let paths = published_drive_paths(sync_dir, drive_letter);
            DriveSyncInfo {
                drive_letter,
                mft_output_path: paths.mft_path,
                index_output_path: paths.base_index_path,
                overlay_output_path: paths.overlay_index_path,
                checkpoint_output_path: paths.checkpoint_path,
            }
        })
        .collect::<Vec<_>>();

    drive_infos.sort_by_key(|info| info.drive_letter);

    if drive_infos.is_empty() {
        bail!("No drives matched the requested drive set");
    }

    Ok(drive_infos)
}
