use crate::cli::command::sync::sync_cli::IfExistsOutputBehaviour;
use crate::sync_dir::try_get_sync_dir;
use eyre::bail;
use eyre::eyre;
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

#[derive(Debug, Clone)]
pub struct DriveSnapshot {
    pub drive_letter: char,
    pub bytes: Vec<u8>,
}

pub fn resolve_drive_infos(
    drive_letter_pattern: &DriveLetterPattern,
    overwrite_existing: &IfExistsOutputBehaviour,
) -> eyre::Result<Vec<DriveSyncInfo>> {
    let sync_dir = try_get_sync_dir()?;

    info!("Syncing in directory: {}", sync_dir.display());
    create_dir_all(&sync_dir)?;

    let drives = drive_letter_pattern
        .into_drive_letters()?
        .into_iter()
        .filter_map(|drive_letter| {
            let drive_output_path = sync_dir.join(format!("{drive_letter}.mft"));
            match (drive_output_path.exists(), overwrite_existing) {
                (false, _) => Some(Ok((drive_letter, drive_output_path))),
                (true, IfExistsOutputBehaviour::Skip) => None,
                (true, IfExistsOutputBehaviour::Overwrite) => {
                    Some(Ok((drive_letter, drive_output_path)))
                }
                (true, IfExistsOutputBehaviour::Abort) => Some(Err(eyre!(
                    "Aborting sync: {} already exists",
                    drive_output_path.display()
                ))),
            }
        })
        .collect::<eyre::Result<Vec<_>>>()?;

    let mut drive_infos = Vec::new();
    for (drive_letter, drive_output_path) in drives {
        drive_infos.push(DriveSyncInfo {
            drive_letter,
            mft_output_path: drive_output_path,
            index_output_path: sync_dir.join(format!("{drive_letter}.mft_search_index")),
        });
    }

    drive_infos.sort_by_key(|info| info.drive_letter);

    if drive_infos.is_empty() {
        bail!("No drives matched the pattern: {}", drive_letter_pattern);
    }

    Ok(drive_infos)
}
