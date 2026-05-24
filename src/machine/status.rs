use crate::machine::config::{
    MachineConfig, PublishedCheckpoint, load_checkpoint, load_machine_config, published_drive_paths,
};
use crate::machine::security::current_user_sid_string;
use crate::machine::service::{WindowsServiceState, query_service_state};
use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;
use teamy_windows::storage::DriveLetterPattern;

#[derive(Debug, Clone)]
pub struct MachineDriveStatus {
    pub drive_letter: char,
    pub mft_path: PathBuf,
    pub mft_modified_at: Option<SystemTime>,
    pub base_index_path: PathBuf,
    pub base_index_modified_at: Option<SystemTime>,
    pub overlay_index_path: PathBuf,
    pub overlay_index_modified_at: Option<SystemTime>,
    pub checkpoint_path: PathBuf,
    pub checkpoint_modified_at: Option<SystemTime>,
    pub checkpoint: Option<PublishedCheckpoint>,
}

#[derive(Debug, Clone)]
pub struct MachineStatus {
    pub config: Option<MachineConfig>,
    pub service_state: WindowsServiceState,
    pub current_user_sid: Option<String>,
    pub owner_access: bool,
    pub drives: Vec<MachineDriveStatus>,
}

/// # Errors
///
/// Returns an error if the machine config or checkpoint files cannot be read.
pub fn load_machine_status(
    drive_letter_pattern: &DriveLetterPattern,
) -> eyre::Result<MachineStatus> {
    let config = load_machine_config()?;
    let current_user_sid = current_user_sid_string().ok();
    let (service_state, owner_access, drives) = if let Some(config) = &config {
        let service_state = query_service_state(&config.service_name)?;
        let owner_access = current_user_sid
            .as_deref()
            .is_some_and(|sid| sid == config.owner_sid);
        let drives = drive_letter_pattern
            .into_drive_letters()?
            .into_iter()
            .map(|drive_letter| {
                let paths = published_drive_paths(&config.cache_root, drive_letter);
                let checkpoint = load_checkpoint(&paths.checkpoint_path)?;
                Ok(MachineDriveStatus {
                    drive_letter,
                    mft_modified_at: modified_at(&paths.mft_path)?,
                    base_index_modified_at: modified_at(&paths.base_index_path)?,
                    overlay_index_modified_at: modified_at(&paths.overlay_index_path)?,
                    checkpoint_modified_at: modified_at(&paths.checkpoint_path)?,
                    mft_path: paths.mft_path,
                    base_index_path: paths.base_index_path,
                    overlay_index_path: paths.overlay_index_path,
                    checkpoint_path: paths.checkpoint_path,
                    checkpoint,
                })
            })
            .collect::<eyre::Result<Vec<_>>>()?;
        (service_state, owner_access, drives)
    } else {
        (WindowsServiceState::Missing, false, Vec::new())
    };

    Ok(MachineStatus {
        config,
        service_state,
        current_user_sid,
        owner_access,
        drives,
    })
}

fn modified_at(path: &std::path::Path) -> eyre::Result<Option<SystemTime>> {
    if !path.is_file() {
        return Ok(None);
    }
    Ok(Some(fs::metadata(path)?.modified()?))
}
