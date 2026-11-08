use eyre::WrapErr;
use std::fs::File;
use std::fs::OpenOptions;
use std::os::windows::fs::OpenOptionsExt;
use std::path::Path;

/// Exclusivity among cooperating full-sync operations in one directory.
/// Dropping the guard closes the OS handle; the lock file remains for reuse.
#[derive(Debug)]
#[must_use = "hold the guard for the duration of the directory operation"]
pub struct DirectoryExclusivityGuard {
    _lock_file: File,
}

impl DirectoryExclusivityGuard {
    /// Acquires exclusivity in an existing directory without waiting.
    ///
    /// # Errors
    ///
    /// Returns an error if another guard holds the lock or the lock file cannot be opened.
    pub fn acquire(directory: &Path) -> eyre::Result<Self> {
        let lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(directory.join(".full-sync.lock"))
            .wrap_err("Another full sync is active for this cache, or its lock cannot be opened")?;

        Ok(Self {
            _lock_file: lock_file,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::DirectoryExclusivityGuard;

    #[test]
    fn overlapping_acquisition_is_rejected_until_guard_is_dropped() -> eyre::Result<()> {
        let temp = tempfile::tempdir()?;
        let first = DirectoryExclusivityGuard::acquire(temp.path())?;
        let _error = DirectoryExclusivityGuard::acquire(temp.path())
            .expect_err("overlapping directory acquisition should fail");
        drop(first);
        let _next = DirectoryExclusivityGuard::acquire(temp.path())?;
        Ok(())
    }

    #[test]
    fn different_directories_can_be_held_simultaneously() -> eyre::Result<()> {
        let first = tempfile::tempdir()?;
        let second = tempfile::tempdir()?;
        let _first_guard = DirectoryExclusivityGuard::acquire(first.path())?;
        let _second_guard = DirectoryExclusivityGuard::acquire(second.path())?;
        Ok(())
    }

    #[test]
    fn existing_lock_file_is_preserved_for_reuse() -> eyre::Result<()> {
        let temp = tempfile::tempdir()?;
        let lock_path = temp.path().join(".full-sync.lock");
        std::fs::write(&lock_path, b"existing lock file")?;

        let guard = DirectoryExclusivityGuard::acquire(temp.path())?;
        drop(guard);

        assert_eq!(std::fs::read(lock_path)?, b"existing lock file");
        let _next = DirectoryExclusivityGuard::acquire(temp.path())?;
        Ok(())
    }
}
