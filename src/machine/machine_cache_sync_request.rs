use crate::domain::DirectoryExclusivityGuard;
use crate::domain::DriveLetter;
use crate::domain::ParallelismLimit;
use crate::machine::DriveArtifactInfoCollection;
use crate::machine::MachineCacheSyncResult;
use crate::machine::config::PublishedCheckpoint;
use crate::machine::config::current_unix_ms;
use crate::machine::config::save_checkpoint;
use crate::machine::usn::JournalCursor;
use crate::machine::usn::VolumeUsnJournalHandle;
use crate::search_index::format::SEARCH_INDEX_VERSION;
use crate::search_index::format::SearchIndexHeader;
use crate::search_index::search_index_bytes::SearchIndexBytesMut;
use crate::sync::IfExistsOutputBehaviour;
use crate::sync::SyncDecision;
use crate::sync::SyncWork;
use crate::windows_utils::elevation::ensure_elevated;
use rustc_hash::FxHashMap;
use std::path::PathBuf;
use teamy_cancellation::CancellationScope;
use tracing::warn;
use uom::si::information::byte;
use uom::si::usize::Information;

#[derive(Debug, Clone)]
pub struct MachineCacheSyncRequest {
    pub sync_dir: PathBuf,
    pub drive_letters: Vec<DriveLetter>,
    pub if_exists: IfExistsOutputBehaviour,
    pub parallel: ParallelismLimit,
    pub console_pid: Option<u32>,
}

impl MachineCacheSyncRequest {
    /// Run cache synchronization on a local async runtime.
    ///
    /// # Errors
    ///
    /// Returns an error if the runtime cannot start or synchronization fails.
    pub fn execute_blocking(
        self,
        cancel: &CancellationScope,
    ) -> eyre::Result<MachineCacheSyncResult> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(self.execute(cancel))
    }

    /// Synchronize snapshots, indexes and their publication sidecars.
    ///
    /// # Errors
    ///
    /// Returns an error if elevation, cache access, preflight, acquisition,
    /// indexing or sidecar publication fails.
    pub async fn execute(self, cancel: &CancellationScope) -> eyre::Result<MachineCacheSyncResult> {
        ensure_elevated(self.console_pid)?;
        std::fs::create_dir_all(&self.sync_dir)?;
        let _directory_guard = DirectoryExclusivityGuard::acquire(&self.sync_dir)?;
        let drive_infos = DriveArtifactInfoCollection::resolve(
            &self.sync_dir,
            self.drive_letters.iter().copied(),
        )?;
        let work = SyncWork::prepare(drive_infos, self.if_exists)?;
        let mut live_drives = Vec::new();
        let mut snapshot_cursors = FxHashMap::default();
        let mut skipped_drives = Vec::new();
        // Capture cursors before acquisition so live refresh can replay changes made during it.
        for (&drive, (_, decision)) in work.iter() {
            if *decision != SyncDecision::MftAndIndex {
                continue;
            }
            match VolumeUsnJournalHandle::open(drive).and_then(|journal| journal.query_cursor()) {
                Ok(cursor) => {
                    live_drives.push(drive);
                    snapshot_cursors.insert(drive, cursor);
                }
                Err(error) => {
                    let message = format!("{error:#}");
                    warn!(
                        drive = %drive,
                        error = %message,
                        "USN journal unavailable; syncing snapshot without live updates"
                    );
                    skipped_drives.push((drive, message));
                }
            }
        }
        for (&drive, (_, decision)) in work.iter() {
            if *decision == SyncDecision::Skip {
                skipped_drives.push((drive, String::from("Existing base index retained")));
            }
        }
        work.execute(self.parallel, self.console_pid, cancel)
            .await?;
        Self::publish_sidecars(&work, &snapshot_cursors)?;

        Ok(MachineCacheSyncResult {
            synced_drives: work
                .iter()
                .filter_map(|(&drive, (_, decision))| {
                    (*decision != SyncDecision::Skip).then_some(drive)
                })
                .collect(),
            live_drives,
            skipped_drives,
        })
    }

    fn publish_sidecars(
        work: &SyncWork,
        snapshot_cursors: &FxHashMap<DriveLetter, JournalCursor>,
    ) -> eyre::Result<()> {
        for (info, decision) in work.values() {
            if *decision != SyncDecision::MftAndIndex {
                continue;
            }
            SearchIndexBytesMut::from_rows(
                SearchIndexHeader::new(info.drive_letter, Information::new::<byte>(0), 0),
                &[],
            )?
            .write_to_path(&info.overlay_index_path)?;
            let cursor = snapshot_cursors.get(&info.drive_letter).copied();
            let checkpoint = if let Some(cursor) = cursor {
                PublishedCheckpoint {
                    drive_letter: info.drive_letter,
                    volume_serial_number: None,
                    journal_id: Some(cursor.journal_id),
                    snapshot_usn: Some(cursor.next_usn),
                    last_usn: Some(cursor.next_usn),
                    published_at_unix_ms: current_unix_ms(),
                    overlay_row_count: 0,
                    base_index_version: SEARCH_INDEX_VERSION,
                }
            } else {
                PublishedCheckpoint {
                    published_at_unix_ms: current_unix_ms(),
                    ..PublishedCheckpoint::empty(info.drive_letter, SEARCH_INDEX_VERSION)
                }
            };
            save_checkpoint(&info.checkpoint_path, &checkpoint)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::MachineCacheSyncRequest;
    use crate::cancellation::CancellationToken;
    use crate::domain::DriveLetter;
    use crate::machine::DriveArtifactInfo;
    use crate::machine::DriveArtifactInfoCollection;
    use crate::machine::config::PublishedCheckpoint;
    use crate::machine::config::load_checkpoint;
    use crate::machine::config::save_checkpoint;
    use crate::machine::usn::JournalCursor;
    use crate::search_index::format::SearchIndexHeader;
    use crate::search_index::format::SearchIndexPathRow;
    use crate::search_index::search_index_bytes::SearchIndexBytesMut;
    use crate::sync::IfExistsOutputBehaviour;
    use crate::sync::SyncDecision;
    use crate::sync::SyncWork;
    use rustc_hash::FxHashMap;
    use uom::si::information::byte;
    use uom::si::usize::Information;

    #[tokio::test]
    async fn skipped_snapshots_preserve_sidecars_and_cached_mft() -> eyre::Result<()> {
        CancellationToken::new()
            .with_async_scope(|scope| async move {
                for (mft_exists, index_exists) in [(true, true), (false, true), (true, false)] {
                    let temp = tempfile::tempdir()?;
                    let paths = DriveArtifactInfo::new(temp.path(), DriveLetter::C);
                    // A cached stream containing one unused record builds an empty base index.
                    let mut mft = vec![0; 1024];
                    mft[0x1c..0x20].copy_from_slice(&1024u32.to_le_bytes());
                    if mft_exists {
                        std::fs::write(&paths.mft_path, &mft)?;
                    }
                    if index_exists {
                        std::fs::write(&paths.base_index_path, b"retained base index")?;
                    }
                    SearchIndexBytesMut::from_rows(
                        SearchIndexHeader::new(DriveLetter::C, Information::new::<byte>(1024), 1),
                        &[SearchIndexPathRow {
                            path: String::from(r"C:\added.txt").into(),
                            has_deleted_entries: false,
                        }],
                    )?
                    .write_to_path(&paths.overlay_index_path)?;
                    let checkpoint = PublishedCheckpoint {
                        journal_id: Some(7),
                        snapshot_usn: Some(100),
                        last_usn: Some(150),
                        overlay_row_count: 1,
                        ..PublishedCheckpoint::empty(
                            DriveLetter::C,
                            crate::search_index::format::SEARCH_INDEX_VERSION,
                        )
                    };
                    save_checkpoint(&paths.checkpoint_path, &checkpoint)?;
                    let overlay_bytes = std::fs::read(&paths.overlay_index_path)?;
                    let checkpoint_bytes = std::fs::read(&paths.checkpoint_path)?;
                    let infos =
                        DriveArtifactInfoCollection::resolve(temp.path(), [DriveLetter::C])?;
                    let work = SyncWork::prepare(infos, IfExistsOutputBehaviour::Skip)?;
                    let expected_decision = if index_exists {
                        SyncDecision::Skip
                    } else {
                        SyncDecision::Index
                    };
                    assert_eq!(work[&DriveLetter::C].1, expected_decision);
                    work.execute(1.into(), None, &scope).await?;
                    assert_eq!(work[&DriveLetter::C].1, expected_decision);
                    let cursors = FxHashMap::from_iter([(
                        DriveLetter::C,
                        JournalCursor {
                            journal_id: 7,
                            first_usn: 0,
                            next_usn: 999,
                            lowest_valid_usn: 0,
                            max_usn: u64::MAX,
                        },
                    )]);
                    MachineCacheSyncRequest::publish_sidecars(&work, &cursors)?;
                    assert_eq!(std::fs::read(&paths.overlay_index_path)?, overlay_bytes);
                    assert_eq!(std::fs::read(&paths.checkpoint_path)?, checkpoint_bytes);
                    if mft_exists {
                        assert_eq!(std::fs::read(&paths.mft_path)?, mft);
                    } else {
                        assert!(!paths.mft_path.exists());
                    }
                    if index_exists {
                        assert_eq!(
                            std::fs::read(&paths.base_index_path)?,
                            b"retained base index"
                        );
                    } else {
                        assert!(paths.base_index_path.exists());
                    }
                }
                Ok(())
            })
            .await
    }

    #[test]
    fn snapshot_publication_only_advances_fresh_drives() -> eyre::Result<()> {
        let temp = tempfile::tempdir()?;
        let old_paths = DriveArtifactInfo::new(temp.path(), DriveLetter::C);
        std::fs::write(&old_paths.mft_path, b"old MFT")?;
        std::fs::write(&old_paths.base_index_path, b"old base")?;
        std::fs::write(&old_paths.overlay_index_path, b"old overlay")?;
        std::fs::write(&old_paths.checkpoint_path, b"old checkpoint")?;
        let infos = DriveArtifactInfoCollection::resolve(
            temp.path(),
            [DriveLetter::C, DriveLetter::D, DriveLetter::E],
        )?;
        let work = SyncWork::prepare(infos, IfExistsOutputBehaviour::Skip)?;
        assert_eq!(
            work.iter()
                .map(|(&drive, (_, decision))| (drive, *decision))
                .collect::<Vec<_>>(),
            [
                (DriveLetter::C, SyncDecision::Skip),
                (DriveLetter::D, SyncDecision::MftAndIndex),
                (DriveLetter::E, SyncDecision::MftAndIndex),
            ]
        );
        let cursors = FxHashMap::from_iter([(
            DriveLetter::D,
            JournalCursor {
                journal_id: 9,
                first_usn: 0,
                next_usn: 200,
                lowest_valid_usn: 0,
                max_usn: u64::MAX,
            },
        )]);
        // The raw-read stage is omitted; this test exercises sidecar publication after success.
        MachineCacheSyncRequest::publish_sidecars(&work, &cursors)?;
        assert_eq!(
            std::fs::read(&old_paths.overlay_index_path)?,
            b"old overlay"
        );
        assert_eq!(
            std::fs::read(&old_paths.checkpoint_path)?,
            b"old checkpoint"
        );
        let fresh_paths = DriveArtifactInfo::new(temp.path(), DriveLetter::D);
        let fresh = load_checkpoint(&fresh_paths.checkpoint_path)?.unwrap();
        assert_eq!(fresh.journal_id, Some(9));
        assert_eq!(fresh.snapshot_usn, Some(200));
        assert_eq!(fresh.last_usn, Some(200));
        assert_eq!(fresh.overlay_row_count, 0);
        let snapshot_only =
            load_checkpoint(&DriveArtifactInfo::new(temp.path(), DriveLetter::E).checkpoint_path)?
                .unwrap();
        assert_eq!(snapshot_only.snapshot_usn, None);
        assert_eq!(snapshot_only.journal_id, None);
        Ok(())
    }

    #[test]
    fn preparation_preserves_skip_overwrite_and_abort_behavior() -> eyre::Result<()> {
        for (mft_exists, index_exists) in
            [(false, false), (true, false), (false, true), (true, true)]
        {
            let temp = tempfile::tempdir()?;
            let paths = DriveArtifactInfo::new(temp.path(), DriveLetter::C);
            if mft_exists {
                std::fs::write(&paths.mft_path, b"MFT")?;
            }
            if index_exists {
                std::fs::write(&paths.base_index_path, b"base")?;
            }
            for policy in [
                IfExistsOutputBehaviour::Skip,
                IfExistsOutputBehaviour::Overwrite,
                IfExistsOutputBehaviour::Abort,
            ] {
                let infos = DriveArtifactInfoCollection::resolve(temp.path(), [DriveLetter::C])?;
                let result = SyncWork::prepare(infos, policy);
                if policy == IfExistsOutputBehaviour::Abort && (mft_exists || index_exists) {
                    let error = result.unwrap_err();
                    assert!(error.to_string().contains("Aborting sync"));
                } else {
                    let work = result?;
                    let expected_decision = match (policy, mft_exists, index_exists) {
                        (IfExistsOutputBehaviour::Skip, _, true) => SyncDecision::Skip,
                        (IfExistsOutputBehaviour::Skip, true, false) => SyncDecision::Index,
                        _ => SyncDecision::MftAndIndex,
                    };
                    assert_eq!(work[&DriveLetter::C].1, expected_decision);
                    assert_eq!(work[&DriveLetter::C].0, paths);
                }
            }
            if mft_exists {
                assert_eq!(std::fs::read(&paths.mft_path)?, b"MFT");
            }
            if index_exists {
                assert_eq!(std::fs::read(&paths.base_index_path)?, b"base");
            }
        }
        Ok(())
    }
}
