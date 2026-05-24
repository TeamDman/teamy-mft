use crate::paths::EnsureParentDirExists;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const MACHINE_ROOT_DIR_NAME: &str = "teamy_mft";
pub const MACHINE_CONFIG_FILE_NAME: &str = "machine_config.json";
pub const DEFAULT_SERVICE_NAME: &str = "teamy-mft-daemon";
pub const DEFAULT_PIPE_NAME: &str = r"\\.\pipe\teamy-mft-daemon";
pub const DEFAULT_IDLE_TIMEOUT_SECS: u64 = 300;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineConfig {
    pub version: u32,
    pub owner_sid: String,
    pub cache_root: PathBuf,
    pub pipe_name: String,
    pub service_name: String,
    pub idle_timeout_secs: u64,
}

impl MachineConfig {
    #[must_use]
    pub fn new(owner_sid: String, cache_root: Option<PathBuf>) -> Self {
        Self {
            version: 1,
            owner_sid,
            cache_root: cache_root.unwrap_or_else(default_cache_root),
            pipe_name: String::from(DEFAULT_PIPE_NAME),
            service_name: String::from(DEFAULT_SERVICE_NAME),
            idle_timeout_secs: DEFAULT_IDLE_TIMEOUT_SECS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedCheckpoint {
    pub drive_letter: char,
    pub volume_serial_number: Option<u32>,
    pub journal_id: Option<u64>,
    pub snapshot_usn: Option<u64>,
    pub last_usn: Option<u64>,
    pub published_at_unix_ms: u64,
    pub overlay_row_count: u64,
    pub base_index_version: u16,
}

impl PublishedCheckpoint {
    #[must_use]
    pub fn empty(drive_letter: char, base_index_version: u16) -> Self {
        Self {
            drive_letter,
            volume_serial_number: None,
            journal_id: None,
            snapshot_usn: None,
            last_usn: None,
            published_at_unix_ms: current_unix_ms(),
            overlay_row_count: 0,
            base_index_version,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedDrivePaths {
    pub drive_letter: char,
    pub mft_path: PathBuf,
    pub base_index_path: PathBuf,
    pub overlay_index_path: PathBuf,
    pub checkpoint_path: PathBuf,
}

#[must_use]
pub fn program_data_dir() -> PathBuf {
    std::env::var_os("PROGRAMDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
}

#[must_use]
pub fn machine_root_dir() -> PathBuf {
    program_data_dir().join(MACHINE_ROOT_DIR_NAME)
}

#[must_use]
pub fn machine_config_path() -> PathBuf {
    machine_root_dir().join(MACHINE_CONFIG_FILE_NAME)
}

#[must_use]
pub fn default_cache_root() -> PathBuf {
    machine_root_dir().join("cache")
}

#[must_use]
pub fn published_drive_paths(cache_root: &Path, drive_letter: char) -> PublishedDrivePaths {
    PublishedDrivePaths {
        drive_letter,
        mft_path: cache_root.join(format!("{drive_letter}.mft")),
        base_index_path: cache_root.join(format!("{drive_letter}.mft_search_index")),
        overlay_index_path: cache_root.join(format!("{drive_letter}.mft_overlay_search_index")),
        checkpoint_path: cache_root.join(format!("{drive_letter}.mft_checkpoint.json")),
    }
}

/// # Errors
///
/// Returns an error if the machine config cannot be read or parsed.
pub fn load_machine_config() -> eyre::Result<Option<MachineConfig>> {
    let path = machine_config_path();
    if !path.is_file() {
        return Ok(None);
    }

    let config = serde_json::from_str::<MachineConfig>(&fs::read_to_string(&path)?)
        .map_err(|error| eyre::eyre!("Failed parsing {}: {error}", path.display()))?;
    Ok(Some(config))
}

/// # Errors
///
/// Returns an error if the machine config cannot be written.
pub fn save_machine_config(config: &MachineConfig) -> eyre::Result<()> {
    let path = machine_config_path();
    path.ensure_parent_dir_exists()?;
    fs::write(&path, serde_json::to_vec_pretty(config)?)?;
    Ok(())
}

/// # Errors
///
/// Returns an error if the checkpoint file cannot be read or parsed.
pub fn load_checkpoint(path: &Path) -> eyre::Result<Option<PublishedCheckpoint>> {
    if !path.is_file() {
        return Ok(None);
    }
    let checkpoint = serde_json::from_str::<PublishedCheckpoint>(&fs::read_to_string(path)?)
        .map_err(|error| eyre::eyre!("Failed parsing {}: {error}", path.display()))?;
    Ok(Some(checkpoint))
}

/// # Errors
///
/// Returns an error if the checkpoint file cannot be written.
pub fn save_checkpoint(path: &Path, checkpoint: &PublishedCheckpoint) -> eyre::Result<()> {
    path.ensure_parent_dir_exists()?;
    fs::write(path, serde_json::to_vec_pretty(checkpoint)?)?;
    Ok(())
}

#[must_use]
pub fn current_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
