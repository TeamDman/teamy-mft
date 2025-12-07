use crate::cli::Cli;
use crate::cli::command::Command;
use crate::cli::command::set_sync_dir::SetSyncDirArgs;
use crate::paths::ConfigDirPath;
use crate::paths::EnsureParentDirExists;
use std::fs;
use std::ops::Deref;
use std::path::Path;
use std::path::PathBuf;
use tracing::debug;
use tracing::warn;

pub const SYNC_DIR_ENV: &str = "TEAMY_MFT_SYNC_DIR";

#[derive(Debug)]
pub struct SyncDirPersistencePath {
    path: PathBuf,
}
impl SyncDirPersistencePath {
    pub fn from_config_dir(config_dir: impl AsRef<Path>) -> Self {
        Self {
            path: config_dir.as_ref().join("sync_dir.txt"),
        }
    }
}
impl Deref for SyncDirPersistencePath {
    type Target = PathBuf;
    fn deref(&self) -> &Self::Target {
        &self.path
    }
}
impl AsRef<Path> for SyncDirPersistencePath {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

/// Determine the current sync directory, checking the environment variable and persisted file.
///
/// # Errors
///
/// Returns an error if accessing the config directory or reading the persisted file fails.
pub fn get_sync_dir() -> color_eyre::eyre::Result<Option<PathBuf>> {
    if let Ok(val) = std::env::var(SYNC_DIR_ENV) {
        let trimmed = val.trim();
        if !trimmed.is_empty() {
            debug!(env = SYNC_DIR_ENV, "Using sync dir from env: {}", trimmed);
            return Ok(Some(PathBuf::from(trimmed)));
        }
    }

    let persist_path = SyncDirPersistencePath::from_config_dir(ConfigDirPath::new()?);
    if !persist_path.exists() {
        return Ok(None);
    }

    debug!(
        "Reading sync dir from persisted file: {}",
        persist_path.display()
    );
    let contents = fs::read_to_string(persist_path.as_ref())?;
    let line = contents.trim();
    if line.is_empty() {
        return Ok(None);
    }
    Ok(Some(PathBuf::from(line)))
}

/// Retrieve the sync directory, failing if it is not configured.
///
/// # Errors
///
/// Returns an error if the sync directory is unset, or if retrieving it fails.
pub fn try_get_sync_dir() -> eyre::Result<PathBuf> {
    let Some(sync_dir) = get_sync_dir()? else {
        eyre::bail!(
            "Sync directory is not set. Please set it using the `{}` command.",
            Cli {
                command: Command::SetSyncDir(SetSyncDirArgs { path: None }),
                ..Default::default()
            }
            .display_invocation()
        );
    };
    Ok(sync_dir)
}

/// Persist the sync directory to disk for future runs.
///
/// # Errors
///
/// Returns an error if the config directory cannot be created or the file cannot be written.
pub fn set_sync_dir(path: impl AsRef<Path>) -> color_eyre::eyre::Result<()> {
    if std::env::var(SYNC_DIR_ENV).is_ok() {
        warn!(
            env = SYNC_DIR_ENV,
            "{} is set; it will override the persisted sync dir when reading", SYNC_DIR_ENV
        );
    }

    let persist_path = SyncDirPersistencePath::from_config_dir(ConfigDirPath::new()?);
    persist_path.ensure_parent_dir_exists()?;

    if persist_path.exists() {
        debug!(
            "Overwriting existing sync dir file at {}",
            persist_path.display()
        );
    }

    // UTF-8 single-line text file
    let path_ref = path.as_ref();
    fs::write(persist_path.as_ref(), format!("{}\n", path_ref.display()))?;
    Ok(())
}
