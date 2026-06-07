use crate::machine::config::DEFAULT_SERVICE_NAME;
use crate::machine::config::MachineConfig;
use crate::machine::config::PublishedCheckpoint;
use crate::machine::config::is_access_denied_error;
use crate::machine::config::load_checkpoint;
use crate::machine::config::load_machine_config;
use crate::machine::config::published_drive_paths;
use crate::machine::security::current_user_sid_string;
use crate::machine::service::WindowsServiceState;
use crate::machine::service::query_service_state;
use crate::windows_utils::storage::DriveLetterPattern;
use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

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
    pub warning: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MachineStatus {
    pub config: Option<MachineConfig>,
    pub config_warning: Option<String>,
    pub service_state: WindowsServiceState,
    pub service_warning: Option<String>,
    pub current_user_sid: Option<String>,
    pub owner_access: bool,
    pub drives: Vec<MachineDriveStatus>,
}

#[derive(Debug, Clone)]
pub struct PublishedDriveSummary {
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
    pub warning: Option<String>,
}

/// # Errors
///
/// Returns an error if the machine config or checkpoint files cannot be read.
pub fn load_machine_status(
    drive_letter_pattern: &DriveLetterPattern,
) -> eyre::Result<MachineStatus> {
    let (config, config_warning) = match load_machine_config() {
        Ok(config) => (config, None),
        Err(error) if is_access_denied_error(&error) => (
            None,
            Some(format!(
                "machine config is installed but not readable from this session: {error}"
            )),
        ),
        Err(error) => return Err(error),
    };
    let current_user_sid = current_user_sid_string().ok();
    let (service_state, service_warning, owner_access, drives) = if let Some(config) = &config {
        let (service_state, service_warning) = match query_service_state(&config.service_name) {
            Ok(service_state) => (service_state, None),
            Err(error) if crate::machine::service::is_service_query_access_denied(&error) => (
                WindowsServiceState::Unknown(0),
                Some(format!(
                    "failed to query Windows service {} from this session: {error}",
                    config.service_name
                )),
            ),
            Err(error) => return Err(error),
        };
        let owner_access = current_user_sid
            .as_deref()
            .is_some_and(|sid| sid == config.owner_sid);
        let drives = drive_letter_pattern
            .into_drive_letters()?
            .into_iter()
            .map(|drive_letter| {
                let paths = published_drive_paths(&config.sync_dir, drive_letter);
                let (mft_modified_at, mft_warning) =
                    modified_at(&paths.mft_path, "mft snapshot metadata")?;
                let (base_index_modified_at, base_index_warning) =
                    modified_at(&paths.base_index_path, "base index metadata")?;
                let (overlay_index_modified_at, overlay_index_warning) =
                    modified_at(&paths.overlay_index_path, "overlay index metadata")?;
                let (checkpoint_modified_at, checkpoint_metadata_warning) =
                    modified_at(&paths.checkpoint_path, "checkpoint metadata")?;
                let (checkpoint, checkpoint_warning) =
                    load_checkpoint_status(&paths.checkpoint_path);
                let warning = [
                    mft_warning,
                    base_index_warning,
                    overlay_index_warning,
                    checkpoint_metadata_warning,
                    checkpoint_warning,
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("; ");
                Ok(MachineDriveStatus {
                    drive_letter,
                    mft_modified_at,
                    base_index_modified_at,
                    overlay_index_modified_at,
                    checkpoint_modified_at,
                    mft_path: paths.mft_path,
                    base_index_path: paths.base_index_path,
                    overlay_index_path: paths.overlay_index_path,
                    checkpoint_path: paths.checkpoint_path,
                    checkpoint,
                    warning: if warning.is_empty() {
                        None
                    } else {
                        Some(warning)
                    },
                })
            })
            .collect::<eyre::Result<Vec<_>>>()?;
        (service_state, service_warning, owner_access, drives)
    } else {
        let (service_state, service_warning) = match query_service_state(DEFAULT_SERVICE_NAME) {
            Ok(service_state) => (service_state, None),
            Err(error) if crate::machine::service::is_service_query_access_denied(&error) => (
                WindowsServiceState::Unknown(0),
                Some(format!(
                    "failed to query Windows service {DEFAULT_SERVICE_NAME} from this session: {error}"
                )),
            ),
            Err(_) => (WindowsServiceState::Unknown(0), None),
        };
        (service_state, service_warning, false, Vec::new())
    };

    Ok(MachineStatus {
        config,
        config_warning,
        service_state,
        service_warning,
        current_user_sid,
        owner_access,
        drives,
    })
}

fn modified_at(
    path: &std::path::Path,
    label: &str,
) -> eyre::Result<(Option<SystemTime>, Option<String>)> {
    if !path.is_file() {
        return Ok((None, None));
    }

    match fs::metadata(path).and_then(|metadata| metadata.modified()) {
        Ok(modified_at) => Ok((Some(modified_at), None)),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => Ok((
            None,
            Some(format!(
                "cannot read {label} at {}: {error}",
                path.display()
            )),
        )),
        Err(error) => Err(error.into()),
    }
}

fn load_checkpoint_status(path: &std::path::Path) -> (Option<PublishedCheckpoint>, Option<String>) {
    match load_checkpoint(path) {
        Ok(checkpoint) => (checkpoint, None),
        Err(error) if is_access_denied_error(&error) => (
            None,
            Some(format!(
                "cannot read checkpoint contents at {}: {error}",
                path.display()
            )),
        ),
        Err(error) => (
            None,
            Some(format!(
                "cannot parse checkpoint contents at {}: {error}",
                path.display()
            )),
        ),
    }
}

/// # Errors
///
/// Returns an error if selected drive letters cannot be resolved or if non-permission filesystem errors occur.
pub fn collect_published_drive_summaries(
    sync_dir: &std::path::Path,
    drive_letter_pattern: &DriveLetterPattern,
) -> eyre::Result<Vec<PublishedDriveSummary>> {
    drive_letter_pattern
        .into_drive_letters()?
        .into_iter()
        .map(|drive_letter| {
            let paths = published_drive_paths(sync_dir, drive_letter);
            let (mft_modified_at, mft_warning) =
                modified_at(&paths.mft_path, "mft snapshot metadata")?;
            let (base_index_modified_at, base_index_warning) =
                modified_at(&paths.base_index_path, "base index metadata")?;
            let (overlay_index_modified_at, overlay_index_warning) =
                modified_at(&paths.overlay_index_path, "overlay index metadata")?;
            let (checkpoint_modified_at, checkpoint_metadata_warning) =
                modified_at(&paths.checkpoint_path, "checkpoint metadata")?;
            let (checkpoint, checkpoint_warning) = load_checkpoint_status(&paths.checkpoint_path);
            let warning = [
                mft_warning,
                base_index_warning,
                overlay_index_warning,
                checkpoint_metadata_warning,
                checkpoint_warning,
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("; ");
            Ok(PublishedDriveSummary {
                drive_letter,
                mft_path: paths.mft_path,
                mft_modified_at,
                base_index_path: paths.base_index_path,
                base_index_modified_at,
                overlay_index_path: paths.overlay_index_path,
                overlay_index_modified_at,
                checkpoint_path: paths.checkpoint_path,
                checkpoint_modified_at,
                checkpoint,
                warning: if warning.is_empty() {
                    None
                } else {
                    Some(warning)
                },
            })
        })
        .collect()
}
