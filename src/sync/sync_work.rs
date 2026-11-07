use crate::domain::ParallelismLimit;
use crate::sync::DriveSyncInfo;
use crate::sync::IfExistsOutputBehaviour;
use crate::sync::SyncIndex;
use crate::sync::SyncMft;
use futures::StreamExt;
use std::collections::BTreeSet;
use std::sync::Arc;
use teamy_cancellation::CancellationScope;
use tracing::Span;
use tracing::info_span;

#[derive(Debug)]
pub struct SyncWork {
    pub mft_drives: Vec<DriveSyncInfo>,
    pub index_drives: Vec<DriveSyncInfo>,
}

impl SyncWork {
    /// Select the MFT reads and index builds before any outputs are changed.
    ///
    /// # Errors
    ///
    /// Returns an error when `Abort` encounters an existing MFT or base index.
    pub fn prepare(
        drive_infos: Vec<DriveSyncInfo>,
        if_exists: &IfExistsOutputBehaviour,
    ) -> eyre::Result<Self> {
        let index_drives = SyncIndex::invoke_preflight(drive_infos.clone(), if_exists)?;
        let mut mft_drives = SyncMft::invoke_preflight(drive_infos, if_exists)?;
        // A retained base index must keep its matching MFT snapshot and sidecars.
        mft_drives.retain(|info| {
            index_drives
                .iter()
                .any(|index| index.drive_letter == info.drive_letter)
        });
        Ok(Self {
            mft_drives,
            index_drives,
        })
    }

    /// # Errors
    ///
    /// Returns acquisition, indexing or worker errors after draining all work.
    pub async fn execute(
        self,
        parallel: ParallelismLimit,
        console_pid: Option<u32>,
        cancel: &CancellationScope,
    ) -> eyre::Result<()> {
        let SyncWork {
            mft_drives,
            index_drives,
        } = self;
        let fresh_letters = mft_drives
            .iter()
            .map(|info| info.drive_letter)
            .collect::<BTreeSet<_>>();
        let index_letters = index_drives
            .iter()
            .map(|info| info.drive_letter)
            .collect::<BTreeSet<_>>();
        let retained = index_drives
            .into_iter()
            .filter(|info| !fresh_letters.contains(&info.drive_letter));
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
        let snapshots = SyncMft::invoke(mft_drives, console_pid, cancel.clone())?;
        let ready = futures::stream::select(snapshots, futures::stream::iter(retained.map(Ok)));
        let cancel = cancel.clone();
        Self::build_ready_indexes(ready, pool, move |info| {
            if !index_letters.contains(&info.drive_letter) {
                return Ok(());
            }
            eyre::ensure!(
                !cancel.is_cancelled(),
                "Cancelled before indexing drive {}",
                info.drive_letter
            );
            SyncIndex::invoke_for_mft_path(&info, &cancel)
        })
        .await
    }

    async fn build_ready_indexes<S, F>(
        ready: S,
        pool: Arc<rayon::ThreadPool>,
        build: F,
    ) -> eyre::Result<()>
    where
        S: futures::Stream<Item = eyre::Result<DriveSyncInfo>>,
        F: Fn(DriveSyncInfo) -> eyre::Result<()> + Send + Sync + 'static,
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
                        tokio::task::spawn_blocking(move || {
                            let _parent = parent.enter();
                            let _span = info_span!(
                                "build_cached_search_index_for_drive",
                                drive = %info.drive_letter,
                            )
                            .entered();
                            pool.install(|| build(info))
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
mod tests {
    use super::*;
    use crate::domain::DriveLetter;
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
    #[tokio::test]
    async fn cached_index_outputs_match_at_parallel_one_two_and_auto() -> eyre::Result<()> {
        crate::cancellation::CancellationToken::new()
            .with_async_scope(|scope| async move {
                let mut expected = Vec::new();
                for parallel in [1, 2, 0] {
                    let temp = tempfile::tempdir()?;
                    let infos =
                        crate::sync::drive_sync_info::resolve_drive_infos_in_dir_for_letters(
                            temp.path(),
                            [DriveLetter::C, DriveLetter::D, DriveLetter::G],
                        )?;
                    for info in &infos {
                        std::fs::write(&info.mft_output_path, crate::mft::sample_mft_records())?;
                    }
                    let work = SyncWork::prepare(infos.clone(), &IfExistsOutputBehaviour::Skip)?;
                    assert!(
                        work.mft_drives.is_empty(),
                        "fixture must never open a volume"
                    );
                    work.execute(parallel.into(), None, &scope).await?;
                    let bytes = infos
                        .iter()
                        .map(|info| std::fs::read(&info.index_output_path))
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
    async fn ready_indexes_run_on_distinct_rayon_threads_with_the_resolved_limit()
    -> eyre::Result<()> {
        for parallel in [1, 2] {
            let temp = tempfile::tempdir()?;
            let infos = crate::sync::drive_sync_info::resolve_drive_infos_in_dir_for_letters(
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
            assert!(
                tokio::time::timeout(Duration::from_millis(50), started_rx.recv())
                    .await
                    .is_err()
            );
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
        let mut infos = crate::sync::drive_sync_info::resolve_drive_infos_in_dir_for_letters(
            temp.path(),
            [DriveLetter::C, DriveLetter::G],
        )?;
        let acquisition_info = infos.pop().unwrap();
        let index_info = infos.pop().unwrap();
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
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut job)
                .await
                .is_err()
        );
        drop(release_index);
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut job)
                .await
                .is_err()
        );
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
        let infos = crate::sync::drive_sync_info::resolve_drive_infos_in_dir_for_letters(
            temp.path(),
            [DriveLetter::C, DriveLetter::D],
        )?;
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
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut job)
                .await
                .is_err()
        );
        drop(release_acquisition);
        tokio::time::timeout(Duration::from_secs(5), job).await???;
        assert_eq!(index_count.load(Ordering::SeqCst), 2);
        Ok(())
    }
}
