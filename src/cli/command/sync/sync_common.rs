use crate::cli::command::sync::sync_args::IfExistsOutputBehaviour;
use crate::ntfs::ntfs_drive_handle::get_volume_disk_extents;
use crate::sync_dir::try_get_sync_dir;
use eyre::bail;
use eyre::eyre;
use std::fs::create_dir_all;
use std::path::PathBuf;
use teamy_windows::storage::DriveLetterPattern;
use tracing::info;

#[derive(Debug)]
pub(crate) struct DriveInfo {
    pub(crate) drive_letter: char,
    pub(crate) output_path: PathBuf,
    pub(crate) index_output_path: PathBuf,
    pub(crate) disk_number: u32,
    pub(crate) starting_offset: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct DriveSnapshot {
    pub(crate) drive_letter: char,
    pub(crate) bytes: Vec<u8>,
}

pub(crate) fn resolve_drive_infos(
    drive_pattern: &DriveLetterPattern,
    overwrite_existing: &IfExistsOutputBehaviour,
    include_disk_extents: bool,
) -> eyre::Result<Vec<DriveInfo>> {
    let sync_dir = try_get_sync_dir()?;

    info!("Syncing in directory: {}", sync_dir.display());
    create_dir_all(&sync_dir)?;

    let drives = drive_pattern
        .into_drive_letters()?
        .into_iter()
        .filter_map(|drive_letter| {
            let drive_output_path = sync_dir.join(format!("{drive_letter}.mft"));
            match (drive_output_path.exists(), overwrite_existing) {
                (false, _) => Some(Ok((drive_letter, drive_output_path))),
                (true, IfExistsOutputBehaviour::Skip) => None,
                (true, IfExistsOutputBehaviour::Overwrite) => Some(Ok((drive_letter, drive_output_path))),
                (true, IfExistsOutputBehaviour::Abort) => Some(Err(eyre!(
                    "Aborting sync: {} already exists",
                    drive_output_path.display()
                ))),
            }
        })
        .collect::<eyre::Result<Vec<_>>>()?;

    let mut drive_infos = Vec::new();
    for (drive_letter, drive_output_path) in drives {
        let (disk_number, starting_offset) = if include_disk_extents {
            let extents = get_volume_disk_extents(drive_letter)?;
            let extent = &extents.Extents[0];
            (extent.DiskNumber, extent.StartingOffset)
        } else {
            (0, 0)
        };

        drive_infos.push(DriveInfo {
            drive_letter,
            output_path: drive_output_path,
            index_output_path: sync_dir.join(format!("{drive_letter}.mft_search_index")),
            disk_number,
            starting_offset,
        });
    }

    drive_infos.sort_by_key(|info| (info.disk_number, info.starting_offset));

    if drive_infos.is_empty() {
        bail!("No drives matched the pattern: {}", drive_pattern);
    }

    Ok(drive_infos)
}
