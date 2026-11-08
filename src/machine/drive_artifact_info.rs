use crate::domain::DriveLetter;
use crate::machine::config::CHECKPOINT_FILE_EXTENSION;
use crate::machine::config::MFT_CACHE_FILE_EXTENSION;
use crate::machine::config::OVERLAY_SEARCH_INDEX_FILE_EXTENSION;
use crate::machine::config::SEARCH_INDEX_FILE_EXTENSION;
use eyre::Context;
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveArtifactInfo {
    pub drive_letter: DriveLetter,
    pub mft_path: PathBuf,
    pub base_index_path: PathBuf,
    pub overlay_index_path: PathBuf,
    pub checkpoint_path: PathBuf,
}

impl DriveArtifactInfo {
    #[must_use]
    pub fn new(sync_dir: &Path, drive_letter: DriveLetter) -> Self {
        Self {
            drive_letter,
            mft_path: sync_dir.join(format!("{drive_letter}{MFT_CACHE_FILE_EXTENSION}")),
            base_index_path: sync_dir.join(format!("{drive_letter}{SEARCH_INDEX_FILE_EXTENSION}")),
            overlay_index_path: sync_dir.join(format!(
                "{drive_letter}{OVERLAY_SEARCH_INDEX_FILE_EXTENSION}"
            )),
            checkpoint_path: sync_dir.join(format!("{drive_letter}{CHECKPOINT_FILE_EXTENSION}")),
        }
    }

    /// Require an existing base search index file.
    ///
    /// # Errors
    ///
    /// Returns an error if the index is missing, is not a file or cannot be inspected.
    pub fn require_base_search_index(&self) -> eyre::Result<()> {
        match std::fs::metadata(&self.base_index_path) {
            Ok(metadata) => eyre::ensure!(
                metadata.is_file(),
                "Search index path {} is not a file",
                self.base_index_path.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                eyre::bail!(
                    "Fast query requires {}. Run `teamy-mft sync --drive-letter-pattern {}` first.",
                    self.base_index_path.display(),
                    self.drive_letter
                );
            }
            Err(error) => {
                return Err(error).wrap_err_with(|| {
                    format!(
                        "Failed inspecting base search index for drive {} at {}",
                        self.drive_letter,
                        self.base_index_path.display()
                    )
                });
            }
        }
        Ok(())
    }
}
