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

pub fn get_sync_dir() -> color_eyre::eyre::Result<Option<PathBuf>> {
    if let Ok(val) = std::env::var(SYNC_DIR_ENV) {
        let trimmed = val.trim();
        if !trimmed.is_empty() {
            return Ok(Some(PathBuf::from(trimmed)));
        }
    }

    let persist_path = SyncDirPersistencePath::from_config_dir(ConfigDirPath::new()?);
    if !persist_path.exists() {
        return Ok(None);
    }

    let contents = fs::read_to_string(persist_path.as_ref())?;
    let line = contents.trim();
    if line.is_empty() {
        return Ok(None);
    }
    Ok(Some(PathBuf::from(line)))
}

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

pub fn set_sync_dir(path: PathBuf) -> color_eyre::eyre::Result<()> {
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
    fs::write(persist_path.as_ref(), format!("{}\n", path.display()))?;
    Ok(())
}
