use crate::windows_utils::storage::DriveLetterPattern;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::SystemTime;

/// Cached status for a single drive's `.mft` and `.mft_search_index` files.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DriveCacheStatus {
    pub drive_letter: char,
    pub mft_path: PathBuf,
    pub mft_modified_at: Option<SystemTime>,
    pub index_path: PathBuf,
    pub index_modified_at: Option<SystemTime>,
}

impl DriveCacheStatus {
    #[must_use]
    pub fn query_ready_at(&self) -> Option<SystemTime> {
        match (self.mft_modified_at, self.index_modified_at) {
            (Some(mft_modified_at), Some(index_modified_at)) => {
                Some(mft_modified_at.min(index_modified_at))
            }
            _ => None,
        }
    }

    #[must_use]
    pub fn is_query_ready(&self) -> bool {
        self.query_ready_at().is_some()
    }

    #[must_use]
    pub fn query_ready_age(&self, now: SystemTime) -> Option<Duration> {
        self.query_ready_at()
            .map(|query_ready_at| now.duration_since(query_ready_at).unwrap_or(Duration::ZERO))
    }
}

/// Query-cache freshness across the configured drive set.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamyMftStatus {
    pub sync_dir: PathBuf,
    pub drives: Vec<DriveCacheStatus>,
}

impl TeamyMftStatus {
    /// # Errors
    ///
    /// Returns an error if the machine cache is unavailable, drive letters cannot be resolved,
    /// or file metadata cannot be read.
    pub fn load_all_drives() -> eyre::Result<Self> {
        Self::load(&DriveLetterPattern::default())
    }

    /// # Errors
    ///
    /// Returns an error if the machine cache is unavailable, drive letters cannot be resolved,
    /// or file metadata cannot be read.
    pub fn load(drive_letter_pattern: &DriveLetterPattern) -> eyre::Result<Self> {
        let sync_dir = crate::machine::config::load_sync_dir_from_config()?;
        let drive_letters = drive_letter_pattern.into_drive_letters()?;
        Self::from_sync_dir_and_drive_letters(&sync_dir, drive_letters)
    }

    /// # Errors
    ///
    /// Returns an error if cached file metadata cannot be read.
    pub fn from_sync_dir_and_drive_letters(
        sync_dir: &Path,
        drive_letters: impl IntoIterator<Item = char>,
    ) -> eyre::Result<Self> {
        let mut drives = drive_letters
            .into_iter()
            .map(|drive_letter| DriveCacheStatus {
                drive_letter,
                mft_path: sync_dir.join(format!("{drive_letter}.mft")),
                mft_modified_at: None,
                index_path: sync_dir.join(format!("{drive_letter}.mft_search_index")),
                index_modified_at: None,
            })
            .collect::<Vec<_>>();

        for drive in &mut drives {
            drive.mft_modified_at = if drive.mft_path.is_file() {
                Some(std::fs::metadata(&drive.mft_path)?.modified()?)
            } else {
                None
            };
            drive.index_modified_at = if drive.index_path.is_file() {
                Some(std::fs::metadata(&drive.index_path)?.modified()?)
            } else {
                None
            };
        }

        drives.sort_by_key(|drive| drive.drive_letter);

        Ok(Self {
            sync_dir: sync_dir.to_path_buf(),
            drives,
        })
    }

    #[must_use]
    pub fn query_ready_drive_count(&self) -> usize {
        self.drives
            .iter()
            .filter(|drive| drive.is_query_ready())
            .count()
    }

    #[must_use]
    pub fn oldest_query_ready_at(&self) -> Option<SystemTime> {
        self.drives
            .iter()
            .filter_map(DriveCacheStatus::query_ready_at)
            .min()
    }

    #[must_use]
    pub fn newest_query_ready_at(&self) -> Option<SystemTime> {
        self.drives
            .iter()
            .filter_map(DriveCacheStatus::query_ready_at)
            .max()
    }

    #[must_use]
    pub fn oldest_query_ready_age(&self, now: SystemTime) -> Option<Duration> {
        self.oldest_query_ready_at()
            .map(|query_ready_at| now.duration_since(query_ready_at).unwrap_or(Duration::ZERO))
    }

    #[must_use]
    pub fn newest_query_ready_age(&self, now: SystemTime) -> Option<Duration> {
        self.newest_query_ready_at()
            .map(|query_ready_at| now.duration_since(query_ready_at).unwrap_or(Duration::ZERO))
    }

    /// # Errors
    ///
    /// Returns an error if any selected drive lacks a query-ready cache or if the oldest
    /// query-ready cache is older than `max_age`.
    pub fn assert_query_ready_not_older_than(
        &self,
        max_age: Duration,
        now: SystemTime,
    ) -> eyre::Result<()> {
        let missing_query_ready = self
            .drives
            .iter()
            .filter(|drive| !drive.is_query_ready())
            .map(|drive| drive.drive_letter)
            .collect::<Vec<_>>();
        if !missing_query_ready.is_empty() {
            eyre::bail!(
                "teamy-mft query cache is incomplete for drives: {}",
                missing_query_ready.iter().collect::<String>()
            );
        }

        let stale_drives = self
            .drives
            .iter()
            .filter_map(|drive| {
                let age = drive.query_ready_age(now)?;
                (age > max_age).then_some((drive.drive_letter, age))
            })
            .collect::<Vec<_>>();
        if stale_drives.is_empty() {
            return Ok(());
        }

        let stale_summary = stale_drives
            .into_iter()
            .map(|(drive_letter, age)| {
                format!("{drive_letter} ({})", humantime::format_duration(age))
            })
            .collect::<Vec<_>>()
            .join(", ");
        eyre::bail!(
            "teamy-mft query cache is older than {} for drives: {}",
            humantime::format_duration(max_age),
            stale_summary
        );
    }
}

#[cfg(test)]
mod tests {
    use super::DriveCacheStatus;
    use super::TeamyMftStatus;
    use std::path::Path;
    use std::time::Duration;
    use std::time::SystemTime;

    #[test]
    fn query_ready_at_uses_the_older_artifact_timestamp() {
        let now = SystemTime::now();
        let older = now - Duration::from_secs(20);
        let newer = now - Duration::from_secs(10);
        let drive = DriveCacheStatus {
            drive_letter: 'C',
            mft_path: Path::new("C.mft").to_path_buf(),
            mft_modified_at: Some(newer),
            index_path: Path::new("C.mft_search_index").to_path_buf(),
            index_modified_at: Some(older),
        };

        assert_eq!(drive.query_ready_at(), Some(older));
    }

    #[test]
    fn freshness_assertion_rejects_missing_query_ready_cache() {
        let status = TeamyMftStatus {
            sync_dir: Path::new("G:/sync-root").to_path_buf(),
            drives: vec![DriveCacheStatus {
                drive_letter: 'C',
                mft_path: Path::new("C.mft").to_path_buf(),
                mft_modified_at: Some(SystemTime::now()),
                index_path: Path::new("C.mft_search_index").to_path_buf(),
                index_modified_at: None,
            }],
        };

        let error = status
            .assert_query_ready_not_older_than(Duration::from_secs(60), SystemTime::now())
            .expect_err("missing search index should fail");
        assert!(
            error
                .to_string()
                .contains("teamy-mft query cache is incomplete for drives: C")
        );
    }

    #[test]
    fn load_status_reads_cached_artifact_metadata() {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        std::fs::write(temp_dir.path().join("C.mft"), b"mft").expect("mft file should be written");
        std::fs::write(temp_dir.path().join("C.mft_search_index"), b"index")
            .expect("index file should be written");

        let status = TeamyMftStatus::from_sync_dir_and_drive_letters(temp_dir.path(), ['C'])
            .expect("status should load");

        assert_eq!(status.drives.len(), 1);
        assert!(status.drives[0].mft_modified_at.is_some());
        assert!(status.drives[0].index_modified_at.is_some());
        assert!(status.drives[0].is_query_ready());
    }
}
