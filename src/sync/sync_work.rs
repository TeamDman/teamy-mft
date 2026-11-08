use crate::domain::DriveLetter;
use crate::domain::ParallelismLimit;
use crate::machine::DriveArtifactInfo;
use crate::machine::DriveArtifactInfoCollection;
use crate::sync::IfExistsOutputBehaviour;
use crate::sync::SyncDecision;
use crate::sync::SyncIndex;
use crate::sync::SyncMft;
use futures::StreamExt;
use std::collections::BTreeMap;
use std::ops::Deref;
use std::sync::Arc;
use teamy_cancellation::CancellationScope;
use tracing::Span;
use tracing::debug;
use tracing::info_span;

#[derive(Debug)]
pub struct SyncWork {
    drives: BTreeMap<DriveLetter, (DriveArtifactInfo, SyncDecision)>,
}

impl SyncWork {
    /// Select the MFT reads and index builds before any outputs are changed.
    ///
    /// # Errors
    ///
    /// Returns an error when `Abort` encounters an existing MFT or base index.
    pub fn prepare(
        drive_infos: DriveArtifactInfoCollection,
        if_exists: IfExistsOutputBehaviour,
    ) -> eyre::Result<Self> {
        let mut drives = BTreeMap::new();
        for info in drive_infos {
            let decision = match if_exists {
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
        Ok(Self { drives })
    }

    /// # Errors
    ///
    /// Returns acquisition, indexing or worker errors after draining all work.
    pub async fn execute(
        &self,
        parallel: ParallelismLimit,
        console_pid: Option<u32>,
        cancel: &CancellationScope,
    ) -> eyre::Result<()> {
        let mft_already_exists = self
            .values()
            .filter(|(_, decision)| *decision == SyncDecision::Index)
            .map(|(info, _)| Ok(info.clone()));
        // A private pool bounds Rayon scans, fixups and path resolution as well as drive jobs.
        // Never change the daemon's global Rayon pool or its live-query worker policy.
        let builder = rayon::ThreadPoolBuilder::new();
        let pool = Arc::new(
            match parallel {
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
            self.values().filter_map(|(info, decision)| {
                (*decision == SyncDecision::MftAndIndex).then_some(info)
            }),
            console_pid,
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

impl Deref for SyncWork {
    type Target = BTreeMap<DriveLetter, (DriveArtifactInfo, SyncDecision)>;

    fn deref(&self) -> &Self::Target {
        &self.drives
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::sync::Condvar;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    struct ReleaseWorkersOnDrop(Vec<Arc<(Mutex<bool>, Condvar)>>);

    impl Drop for ReleaseWorkersOnDrop {
        fn drop(&mut self) {
            for gate in &self.0 {
                *gate.0.lock().unwrap() = true;
                gate.1.notify_all();
            }
        }
    }

    #[test]
    fn mixed_drive_decisions_keep_artifacts_attached_and_skips_visible() -> eyre::Result<()> {
        let temp = tempfile::tempdir()?;
        let infos = DriveArtifactInfoCollection::resolve(
            temp.path(),
            [
                DriveLetter::G,
                DriveLetter::E,
                DriveLetter::C,
                DriveLetter::D,
                DriveLetter::C,
            ],
        )?;
        std::fs::write(&infos[&DriveLetter::C].mft_path, b"cached MFT")?;
        std::fs::write(&infos[&DriveLetter::D].base_index_path, b"retained index")?;
        std::fs::write(&infos[&DriveLetter::E].mft_path, b"retained MFT")?;
        std::fs::write(&infos[&DriveLetter::E].base_index_path, b"retained index")?;

        let work = SyncWork::prepare(infos, IfExistsOutputBehaviour::Skip)?;

        assert_eq!(
            work.iter()
                .map(|(&drive, (_, decision))| (drive, *decision))
                .collect::<Vec<_>>(),
            [
                (DriveLetter::C, SyncDecision::Index),
                (DriveLetter::D, SyncDecision::Skip),
                (DriveLetter::E, SyncDecision::Skip),
                (DriveLetter::G, SyncDecision::MftAndIndex),
            ]
        );
        for (&drive, (info, _)) in work.iter() {
            assert_eq!(drive, info.drive_letter);
            assert_eq!(*info, DriveArtifactInfo::new(temp.path(), drive));
        }
        assert!(!work[&DriveLetter::G].0.mft_path.exists());
        assert!(!work[&DriveLetter::D].0.mft_path.exists());
        Ok(())
    }

    #[tokio::test]
    async fn cached_index_outputs_match_at_parallel_one_two_and_auto() -> eyre::Result<()> {
        crate::cancellation::CancellationToken::new()
            .with_async_scope(|scope| async move {
                let mut expected = Vec::new();
                for parallel in [1, 2, 0] {
                    let temp = tempfile::tempdir()?;
                    let infos = DriveArtifactInfoCollection::resolve(
                        temp.path(),
                        [DriveLetter::C, DriveLetter::D, DriveLetter::G],
                    )?;
                    for info in infos.values() {
                        std::fs::write(&info.mft_path, crate::mft::sample_mft_records())?;
                    }
                    let work = SyncWork::prepare(infos, IfExistsOutputBehaviour::Skip)?;
                    assert!(
                        work.values()
                            .all(|(_, decision)| *decision == SyncDecision::Index),
                        "fixture must never open a volume"
                    );
                    work.execute(parallel.into(), None, &scope).await?;
                    let bytes = work
                        .values()
                        .map(|(info, _)| std::fs::read(&info.base_index_path))
                        .collect::<std::io::Result<Vec<_>>>()?;
                    if parallel == 1 {
                        expected = bytes;
                    } else {
                        assert_eq!(bytes, expected);
                    }
                }
                Ok(())
            })
            .await
    }

    #[tokio::test(flavor = "current_thread")]
    async fn index_logs_keep_the_rpc_subscriber_and_correlation_on_rayon() -> eyre::Result<()> {
        use crate::daemon::CorrelationId;
        use crate::machine::daemon_log::DaemonTraceLayer;
        use crate::machine::daemon_log::snapshot_for_correlation;
        use tracing::Instrument;
        use tracing::instrument::WithSubscriber;
        use tracing_subscriber::prelude::*;

        let temp = tempfile::tempdir()?;
        let infos = DriveArtifactInfoCollection::resolve(temp.path(), [DriveLetter::C])?;
        let correlation_id = CorrelationId::new();
        let dispatch =
            tracing::Dispatch::new(tracing_subscriber::registry().with(DaemonTraceLayer));
        let span = tracing::dispatcher::with_default(&dispatch, || {
            tracing::info_span!(
                "daemon_rpc",
                correlation_id = %correlation_id,
                rpc_method = "sync",
            )
        });
        SyncWork::build_ready_indexes(
            futures::stream::iter(infos.into_iter().map(Ok)),
            Arc::new(rayon::ThreadPoolBuilder::new().num_threads(1).build()?),
            |_| {
                tracing::info!("index worker correlation probe");
                Ok(())
            },
        )
        .instrument(span)
        .with_subscriber(dispatch)
        .await?;

        let events = snapshot_for_correlation(&correlation_id);
        let event = events
            .iter()
            .find(|event| event.message == "index worker correlation probe")
            .expect("index logs must reach the RPC subscriber with its correlation ID");
        assert_eq!(event.method, "sync");
        assert!(
            event.spans.iter().any(|span| span.name == "daemon_rpc"),
            "index event must retain its RPC span: {event:#?}"
        );
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn ready_indexes_run_on_distinct_rayon_threads_with_the_resolved_limit()
    -> eyre::Result<()> {
        for parallel in [1, 2] {
            let temp = tempfile::tempdir()?;
            let infos = DriveArtifactInfoCollection::resolve(
                temp.path(),
                [
                    DriveLetter::C,
                    DriveLetter::D,
                    DriveLetter::E,
                    DriveLetter::F,
                    DriveLetter::G,
                ],
            )?;
            let active = Arc::new(AtomicUsize::new(0));
            let peak = Arc::new(AtomicUsize::new(0));
            let completed = Arc::new(AtomicUsize::new(0));
            let gate = Arc::new((Mutex::new(false), Condvar::new()));
            let release = ReleaseWorkersOnDrop(vec![Arc::clone(&gate)]);
            let (started_tx, mut started_rx) = tokio::sync::mpsc::unbounded_channel();
            let job = tokio::spawn(SyncWork::build_ready_indexes(
                futures::stream::iter(infos.into_iter().map(Ok)),
                Arc::new(
                    rayon::ThreadPoolBuilder::new()
                        .num_threads(parallel)
                        .build()?,
                ),
                {
                    let active = Arc::clone(&active);
                    let peak = Arc::clone(&peak);
                    let completed = Arc::clone(&completed);
                    move |_| {
                        let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(current, Ordering::SeqCst);
                        started_tx.send((
                            std::thread::current().id(),
                            rayon::current_thread_index(),
                            rayon::current_num_threads(),
                        ))?;
                        let (_released, wait) = gate
                            .1
                            .wait_timeout_while(
                                gate.0.lock().unwrap(),
                                Duration::from_secs(10),
                                |released| !*released,
                            )
                            .unwrap();
                        active.fetch_sub(1, Ordering::SeqCst);
                        eyre::ensure!(!wait.timed_out(), "worker release timed out");
                        completed.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    }
                },
            ));
            let mut threads = std::collections::HashSet::new();
            let mut workers = BTreeSet::new();
            for _ in 0..parallel {
                let (thread, worker, pool_size) =
                    tokio::time::timeout(Duration::from_secs(5), started_rx.recv())
                        .await?
                        .expect("index worker must report its thread");
                assert_ne!(thread, std::thread::current().id());
                threads.insert(thread);
                workers.insert(worker.expect("index build must run inside Rayon"));
                assert_eq!(pool_size, parallel);
            }
            assert_eq!(threads.len(), parallel);
            assert_eq!(workers.len(), parallel);
            assert_eq!(active.load(Ordering::SeqCst), parallel);
            tokio::time::timeout(Duration::from_millis(50), started_rx.recv())
                .await
                .expect_err("no additional index job may start while all workers are occupied");
            drop(release);
            tokio::time::timeout(Duration::from_secs(5), job).await???;
            assert_eq!(peak.load(Ordering::SeqCst), parallel);
            assert_eq!(active.load(Ordering::SeqCst), 0);
            assert_eq!(completed.load(Ordering::SeqCst), 5);
        }
        Ok(())
    }
    #[tokio::test(flavor = "current_thread")]
    async fn acquisition_failure_drains_real_index_and_acquisition_workers() -> eyre::Result<()> {
        let temp = tempfile::tempdir()?;
        let infos =
            DriveArtifactInfoCollection::resolve(temp.path(), [DriveLetter::C, DriveLetter::G])?;
        let acquisition_info = infos.get(&DriveLetter::G).unwrap().clone();
        let index_info = infos.get(&DriveLetter::C).unwrap().clone();
        let index_gate = Arc::new((Mutex::new(false), Condvar::new()));
        let acquisition_gate = Arc::new((Mutex::new(false), Condvar::new()));
        let release_index = ReleaseWorkersOnDrop(vec![Arc::clone(&index_gate)]);
        let release_acquisition = ReleaseWorkersOnDrop(vec![Arc::clone(&acquisition_gate)]);
        let index_started = Arc::new(tokio::sync::Notify::new());
        let error_ready = Arc::clone(&index_started);
        let (started_tx, mut started_rx) = tokio::sync::mpsc::unbounded_channel();
        let acquisition_started = started_tx.clone();
        let acquisition_finished = Arc::new(AtomicBool::new(false));
        let acquisition_finished_job = Arc::clone(&acquisition_finished);
        let acquisition = futures::stream::once(async move {
            tokio::task::spawn_blocking(move || {
                acquisition_started.send("acquisition")?;
                let (_released, wait) = acquisition_gate
                    .1
                    .wait_timeout_while(
                        acquisition_gate.0.lock().unwrap(),
                        Duration::from_secs(10),
                        |released| !*released,
                    )
                    .unwrap();
                eyre::ensure!(!wait.timed_out(), "acquisition release timed out");
                acquisition_finished_job.store(true, Ordering::SeqCst);
                eyre::Ok(acquisition_info)
            })
            .await
            .map_err(|error| eyre::eyre!(error))?
        });
        let ready = futures::stream::iter([Ok(index_info)])
            .chain(futures::stream::once(async move {
                error_ready.notified().await;
                Err(eyre::eyre!("injected other-drive read failure"))
            }))
            .chain(acquisition);
        let index_finished = Arc::new(AtomicBool::new(false));
        let index_finished_job = Arc::clone(&index_finished);
        let index_count = Arc::new(AtomicUsize::new(0));
        let index_count_job = Arc::clone(&index_count);
        let mut job = tokio::spawn(SyncWork::build_ready_indexes(
            ready,
            Arc::new(rayon::ThreadPoolBuilder::new().num_threads(2).build()?),
            move |info| {
                eyre::ensure!(
                    info.drive_letter == DriveLetter::C,
                    "index started after read failure"
                );
                index_count_job.fetch_add(1, Ordering::SeqCst);
                started_tx.send("index")?;
                index_started.notify_one();
                let (_released, wait) = index_gate
                    .1
                    .wait_timeout_while(
                        index_gate.0.lock().unwrap(),
                        Duration::from_secs(10),
                        |released| !*released,
                    )
                    .unwrap();
                eyre::ensure!(!wait.timed_out(), "index release timed out");
                index_finished_job.store(true, Ordering::SeqCst);
                Ok(())
            },
        ));
        let mut started = BTreeSet::new();
        for _ in 0..2 {
            started.insert(
                tokio::time::timeout(Duration::from_secs(5), started_rx.recv())
                    .await?
                    .unwrap(),
            );
        }
        assert_eq!(started, BTreeSet::from(["acquisition", "index"]));
        tokio::time::timeout(Duration::from_millis(50), &mut job)
            .await
            .expect_err("sync must wait for the active index and acquisition workers");
        drop(release_index);
        tokio::time::timeout(Duration::from_millis(50), &mut job)
            .await
            .expect_err("sync must keep waiting for the acquisition after indexing finishes");
        drop(release_acquisition);
        let error = tokio::time::timeout(Duration::from_secs(5), job)
            .await??
            .unwrap_err();
        assert!(error.to_string().contains("other-drive read failure"));
        assert!(index_finished.load(Ordering::SeqCst));
        assert!(acquisition_finished.load(Ordering::SeqCst));
        assert_eq!(index_count.load(Ordering::SeqCst), 1);
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn ready_index_overlaps_a_blocking_acquisition_on_a_single_thread_runtime()
    -> eyre::Result<()> {
        let temp = tempfile::tempdir()?;
        let infos =
            DriveArtifactInfoCollection::resolve(temp.path(), [DriveLetter::C, DriveLetter::D])?;
        let acquisition_gate = Arc::new((Mutex::new(false), Condvar::new()));
        let release_acquisition = ReleaseWorkersOnDrop(vec![Arc::clone(&acquisition_gate)]);
        let acquisition_thread = Arc::new((Mutex::new(None), Condvar::new()));
        let acquisition_active = Arc::new(AtomicBool::new(false));
        let acquisition_active_job = Arc::clone(&acquisition_active);
        let index_acquisition_thread = Arc::clone(&acquisition_thread);
        let ready = futures::stream::iter(infos)
            .map(move |info| {
                let acquisition_gate = Arc::clone(&acquisition_gate);
                let acquisition_thread = Arc::clone(&acquisition_thread);
                let acquisition_active = Arc::clone(&acquisition_active_job);
                async move {
                    tokio::task::spawn_blocking(move || {
                        if info.drive_letter == DriveLetter::D {
                            acquisition_active.store(true, Ordering::SeqCst);
                            *acquisition_thread.0.lock().unwrap() =
                                Some(std::thread::current().id());
                            acquisition_thread.1.notify_all();
                            let (_released, wait) = acquisition_gate
                                .1
                                .wait_timeout_while(
                                    acquisition_gate.0.lock().unwrap(),
                                    Duration::from_secs(10),
                                    |released| !*released,
                                )
                                .unwrap();
                            acquisition_active.store(false, Ordering::SeqCst);
                            eyre::ensure!(!wait.timed_out(), "acquisition release timed out");
                        } else {
                            let (_started, wait) = acquisition_thread
                                .1
                                .wait_timeout_while(
                                    acquisition_thread.0.lock().unwrap(),
                                    Duration::from_secs(10),
                                    |thread| thread.is_none(),
                                )
                                .unwrap();
                            eyre::ensure!(!wait.timed_out(), "other acquisition did not start");
                        }
                        eyre::Ok(info)
                    })
                    .await
                    .map_err(|error| eyre::eyre!(error))?
                }
            })
            .buffer_unordered(2);
        let (index_started_tx, mut index_started_rx) = tokio::sync::mpsc::unbounded_channel();
        let index_count = Arc::new(AtomicUsize::new(0));
        let index_count_job = Arc::clone(&index_count);
        let mut job = tokio::spawn(SyncWork::build_ready_indexes(
            ready,
            Arc::new(rayon::ThreadPoolBuilder::new().num_threads(1).build()?),
            move |info| {
                eyre::ensure!(
                    rayon::current_thread_index().is_some(),
                    "index must run inside Rayon"
                );
                if info.drive_letter == DriveLetter::C {
                    eyre::ensure!(
                        acquisition_active.load(Ordering::SeqCst),
                        "other acquisition must still be active"
                    );
                    let acquisition_thread = index_acquisition_thread.0.lock().unwrap().unwrap();
                    index_started_tx.send((acquisition_thread, std::thread::current().id()))?;
                }
                index_count_job.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        ));
        let (read_thread, index_thread) =
            tokio::time::timeout(Duration::from_secs(5), index_started_rx.recv())
                .await?
                .expect("ready snapshot must start indexing before the other read finishes");
        assert_ne!(read_thread, index_thread);
        assert_ne!(index_thread, std::thread::current().id());
        tokio::time::timeout(Duration::from_millis(50), &mut job)
            .await
            .expect_err("sync must wait for the drive acquisition still in progress");
        drop(release_acquisition);
        tokio::time::timeout(Duration::from_secs(5), job).await???;
        assert_eq!(index_count.load(Ordering::SeqCst), 2);
        Ok(())
    }
}
