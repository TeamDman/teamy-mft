use directories::ProjectDirs;
use eyre::eyre;
use std::fs;
use std::ops::Deref;
use std::path::Path;
use std::path::PathBuf;

fn project_dirs() -> eyre::Result<ProjectDirs> {
    ProjectDirs::from_path(PathBuf::from("teamy_mft"))
        .ok_or_else(|| eyre!("Could not determine project directories"))
}

#[derive(Debug)]
pub struct ConfigDirPath {
    path: PathBuf,
}
impl ConfigDirPath {
    /// Create a `ConfigDirPath` from the remembered project directories.
    ///
    /// # Errors
    ///
    /// Returns an error if the project directories cannot be determined.
    pub fn new() -> eyre::Result<Self> {
        Ok(Self {
            path: project_dirs()?.config_dir().to_path_buf(),
        })
    }
}
impl Deref for ConfigDirPath {
    type Target = PathBuf;

    fn deref(&self) -> &Self::Target {
        &self.path
    }
}
impl AsRef<Path> for ConfigDirPath {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

// Extension trait to ensure the parent directory of a path exists (useful for file paths)
pub trait EnsureParentDirExists {
    /// Ensure the parent directory exists, creating it if necessary.
    ///
    /// # Errors
    ///
    /// Returns an error if creating the parent directory fails.
    fn ensure_parent_dir_exists(&self) -> eyre::Result<()>;
}
impl<T: AsRef<Path>> EnsureParentDirExists for T {
    /// Ensure the parent directory exists, creating it if needed.
    ///
    /// # Errors
    ///
    /// Returns an error if creating the parent directory fails.
    fn ensure_parent_dir_exists(&self) -> eyre::Result<()> {
        if let Some(parent) = self.as_ref().parent()
            && !parent.exists()
        {
            fs::create_dir_all(parent)?;
        }
        Ok(())
    }
}
