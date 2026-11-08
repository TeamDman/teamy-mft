use crate::domain::DriveLetter;
use crate::machine::DriveArtifactInfo;
use std::collections::BTreeMap;
use std::collections::btree_map::IntoValues;
use std::fs::create_dir_all;
use std::ops::Deref;
use std::path::Path;
use tracing::info;

/// Artifact records keyed by drive letter, in alphabetical order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DriveArtifactInfoCollection {
    entries: BTreeMap<DriveLetter, DriveArtifactInfo>,
}

impl DriveArtifactInfoCollection {
    /// Resolve one artifact record per requested drive.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or no drives are requested.
    pub fn resolve(
        sync_dir: &Path,
        drive_letters: impl IntoIterator<Item = DriveLetter>,
    ) -> eyre::Result<Self> {
        info!("Syncing in directory: {}", sync_dir.display());
        create_dir_all(sync_dir)?;
        let entries = drive_letters
            .into_iter()
            .map(|drive| DriveArtifactInfo::new(sync_dir, drive))
            .collect::<Self>();
        eyre::ensure!(
            !entries.is_empty(),
            "No drives matched the requested drive set"
        );
        Ok(entries)
    }
}

impl Deref for DriveArtifactInfoCollection {
    type Target = BTreeMap<DriveLetter, DriveArtifactInfo>;

    fn deref(&self) -> &Self::Target {
        &self.entries
    }
}

impl FromIterator<DriveArtifactInfo> for DriveArtifactInfoCollection {
    fn from_iter<T: IntoIterator<Item = DriveArtifactInfo>>(iter: T) -> Self {
        Self {
            entries: iter
                .into_iter()
                .map(|info| (info.drive_letter, info))
                .collect(),
        }
    }
}

/// Consuming iteration yields artifact records in drive-letter order.
impl IntoIterator for DriveArtifactInfoCollection {
    type Item = DriveArtifactInfo;
    type IntoIter = IntoValues<DriveLetter, DriveArtifactInfo>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.into_values()
    }
}

#[cfg(test)]
mod tests {
    use super::DriveArtifactInfoCollection;
    use crate::domain::DriveLetter;

    #[test]
    fn resolve_deduplicates_and_orders_drives() -> eyre::Result<()> {
        let temp = tempfile::tempdir()?;
        let artifacts = DriveArtifactInfoCollection::resolve(
            temp.path(),
            [
                DriveLetter::G,
                DriveLetter::C,
                DriveLetter::D,
                DriveLetter::C,
            ],
        )?;
        assert_eq!(
            artifacts.keys().copied().collect::<Vec<_>>(),
            [DriveLetter::C, DriveLetter::D, DriveLetter::G],
        );
        for (drive, info) in artifacts.iter() {
            assert_eq!(*drive, info.drive_letter);
        }
        assert_eq!(
            artifacts
                .into_iter()
                .map(|info| info.drive_letter)
                .collect::<Vec<_>>(),
            [DriveLetter::C, DriveLetter::D, DriveLetter::G],
        );
        Ok(())
    }

    #[test]
    fn empty_resolution_returns_error_after_directory_creation() -> eyre::Result<()> {
        let temp = tempfile::tempdir()?;
        let directory = temp.path().join("cache");
        let error = DriveArtifactInfoCollection::resolve(&directory, [])
            .expect_err("empty initial drive selection should be rejected");
        assert!(error.to_string().contains("No drives matched"));
        assert!(directory.is_dir());
        Ok(())
    }
}
