use crate::domain::DirectoryExclusivityGuard;
use crate::domain::DriveLetter;
use crate::domain::ParallelismLimit;
use crate::machine::DriveArtifactInfo;
use crate::machine::DriveArtifactInfoCollection;
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
use crate::sync::SyncIndex;
use crate::sync::SyncMft;
use crate::sync::SyncResult;
use crate::windows_utils::elevation::ensure_elevated;
use futures::StreamExt;
use rustc_hash::FxHashMap;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use teamy_cancellation::CancellationScope;
use tracing::Span;
use tracing::debug;
use tracing::info_span;
use tracing::warn;
use uom::si::information::byte;
use uom::si::usize::Information;

#[derive(Debug, Clone)]
pub struct SyncRequest {
    pub sync_dir: PathBuf,
    pub drive_letters: Vec<DriveLetter>,
    pub if_exists: IfExistsOutputBehaviour,
    pub parallel: ParallelismLimit,
    pub console_pid: Option<u32>,
}

impl SyncRequest {
    /// Synchronize snapshots, indexes and their publication sidecars.
    ///
    /// # Errors
    ///
    /// Returns an error if elevation, cache access, preflight, acquisition,
    /// indexing or sidecar publication fails.
    pub async fn execute(self, cancel: &CancellationScope) -> eyre::Result<SyncResult> {
        ensure_elevated(self.console_pid)?;
        std::fs::create_dir_all(&self.sync_dir)?;
        let _directory_guard = DirectoryExclusivityGuard::acquire(&self.sync_dir)?;
        let drives = self.prepare()?;
        let mut live_drives = Vec::new();
        let mut snapshot_cursors = FxHashMap::default();
        let mut skipped_drives = Vec::new();
        // Capture cursors before acquisition so live refresh can replay changes made during it.
        for (&drive, (_info, decision)) in &drives {
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
        for (&drive, (_, decision)) in &drives {
            if *decision == SyncDecision::Skip {
                skipped_drives.push((drive, String::from("Existing base index retained")));
            }
        }
        self.execute_drives(&drives, cancel).await?;
        Self::publish_sidecars(&drives, &snapshot_cursors)?;

        Ok(SyncResult {
            synced_drives: drives
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
        drives: &BTreeMap<DriveLetter, (DriveArtifactInfo, SyncDecision)>,
        snapshot_cursors: &FxHashMap<DriveLetter, JournalCursor>,
    ) -> eyre::Result<()> {
        for (info, decision) in drives.values() {
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

    /// Select the MFT reads and index builds before any outputs are changed.
    ///
    /// # Errors
    ///
    /// Returns an error if the cache directory cannot be created, no drives are
    /// requested, or `Abort` encounters an existing MFT or base index.
    fn prepare(&self) -> eyre::Result<BTreeMap<DriveLetter, (DriveArtifactInfo, SyncDecision)>> {
        let drive_infos = DriveArtifactInfoCollection::resolve(
            &self.sync_dir,
            self.drive_letters.iter().copied(),
        )?;
        let mut drives = BTreeMap::new();
        for info in drive_infos {
            let decision = match self.if_exists {
                IfExistsOutputBehaviour::Abort => {
                    for path in [&info.base_index_path, &info.mft_path] {
                        eyre::ensure!(
                            !path.exists(),
                            "Aborting sync: {} already exists",
                            path.display()
                        );
                    }
                    SyncDecision::MftAndIndex
                }
                IfExistsOutputBehaviour::Skip if info.base_index_path.exists() => {
                    // Retain the matching snapshot and sidecars with the base index.
                    debug!(drive = %info.drive_letter, path = %info.base_index_path.display(), "Skipping existing index output");
                    SyncDecision::Skip
                }
                IfExistsOutputBehaviour::Skip if info.mft_path.exists() => {
                    debug!(drive = %info.drive_letter, path = %info.mft_path.display(), "Using existing MFT snapshot for indexing");
                    SyncDecision::Index
                }
                IfExistsOutputBehaviour::Overwrite | IfExistsOutputBehaviour::Skip => {
                    SyncDecision::MftAndIndex
                }
            };
            drives.insert(info.drive_letter, (info, decision));
        }
        Ok(drives)
    }

    /// # Errors
    ///
    /// Returns acquisition, indexing or worker errors after draining all work.
    async fn execute_drives(
        &self,
        drives: &BTreeMap<DriveLetter, (DriveArtifactInfo, SyncDecision)>,
        cancel: &CancellationScope,
    ) -> eyre::Result<()> {
        let mft_already_exists = drives
            .values()
            .filter(|(_, decision)| *decision == SyncDecision::Index)
            .map(|(info, _)| Ok(info.clone()));
        // A private pool bounds Rayon scans, fixups and path resolution as well as drive jobs.
        // Never change the daemon's global Rayon pool or its live-query worker policy.
        let builder = rayon::ThreadPoolBuilder::new();
        let pool = Arc::new(
            match self.parallel {
                ParallelismLimit::Automatic => builder,
                ParallelismLimit::Fixed(limit) => builder.num_threads(limit.get()),
            }
            .build()?,
        );
        // Resolve Rayon's automatic count before limiting concurrent index builds.
        // Passing zero to for_each_concurrent would allow unlimited builds.
        let parallel = pool.current_num_threads();
        tracing::info!(
            parallel,
            "Syncing drives with parallel streaming reads and bounded indexing"
        );
        let snapshots = SyncMft::invoke(
            drives.values().filter_map(|(info, decision)| {
                (*decision == SyncDecision::MftAndIndex).then_some(info)
            }),
            self.console_pid,
            cancel.clone(),
        )?;
        let ready = futures::stream::select(snapshots, futures::stream::iter(mft_already_exists));
        let cancel = cancel.clone();
        Self::build_ready_indexes(ready, pool, move |info| {
            eyre::ensure!(
                !cancel.is_cancelled(),
                "Cancelled before indexing drive {}",
                info.drive_letter
            );
            SyncIndex::build(&info, &cancel)
        })
        .await
    }

    async fn build_ready_indexes<S, F>(
        ready: S,
        pool: Arc<rayon::ThreadPool>,
        build: F,
    ) -> eyre::Result<()>
    where
        S: futures::Stream<Item = eyre::Result<DriveArtifactInfo>>,
        F: Fn(DriveArtifactInfo) -> eyre::Result<()> + Send + Sync + 'static,
    {
        let parallel = pool.current_num_threads();
        let build = Arc::new(build);
        let first_error = Arc::new(std::sync::Mutex::new(None));
        let error_for_jobs = Arc::clone(&first_error);
        // Drain acquisition and await every started task even after the first error.
        ready
            .for_each_concurrent(parallel, move |item| {
                let build = Arc::clone(&build);
                let pool = Arc::clone(&pool);
                let errors = Arc::clone(&error_for_jobs);
                async move {
                    let result = async {
                        let info = item?;
                        if errors
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .is_some()
                        {
                            return Ok(());
                        }
                        let parent = Span::current();
                        let dispatch = tracing::dispatcher::get_default(Clone::clone);
                        tokio::task::spawn_blocking(move || {
                            pool.install(|| {
                                // Rayon runs on another thread; enter the RPC context there.
                                tracing::dispatcher::with_default(&dispatch, || {
                                    let _parent = parent.enter();
                                    let _span = info_span!(
                                        "build_cached_search_index_for_drive",
                                        drive = %info.drive_letter,
                                    )
                                    .entered();
                                    build(info)
                                })
                            })
                        })
                        .await
                        .map_err(|error| eyre::eyre!("Failed joining index task: {error}"))?
                    }
                    .await;
                    if let Err(error) = result {
                        let mut first = errors
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        if first.is_none() {
                            *first = Some(error);
                        }
                    }
                }
            })
            .await;
        match first_error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests;
