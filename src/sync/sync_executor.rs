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
}

/// # Errors
///
/// Returns an error if the sync fails, likely caused by IO problems.
/// Work must be selected with `SyncWork::prepare` before execution.
pub async fn execute_sync(
    work: SyncWork,
    parallel: ParallelismLimit,
    console_pid: Option<u32>,
    cancel: &CancellationScope,
) -> eyre::Result<()> {
    let SyncWork {
        mft_drives,
        index_drives,
    } = work;
    let fresh_letters = mft_drives
        .iter()
        .map(|info| info.drive_letter)
        .collect::<BTreeSet<_>>();
    let index_letters = Arc::new(
        index_drives
            .iter()
            .map(|info| info.drive_letter)
            .collect::<BTreeSet<_>>(),
    );
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
    build_ready_indexes(ready, parallel, move |info| {
        let index_letters = Arc::clone(&index_letters);
        let pool = Arc::clone(&pool);
        let cancel = cancel.clone();
        async move {
            if !index_letters.contains(&info.drive_letter) {
                return Ok(());
            }
            let parent = Span::current();
            tokio::task::spawn_blocking(move || {
                let _parent = parent.enter();
                let _span =
                    info_span!("build_cached_search_index_for_drive", drive = %info.drive_letter)
                        .entered();
                eyre::ensure!(
                    !cancel.is_cancelled(),
                    "Cancelled before indexing drive {}",
                    info.drive_letter
                );
                pool.install(|| SyncIndex::invoke_for_mft_path(&info, &cancel))
            })
            .await
            .map_err(|error| eyre::eyre!("Failed joining index task: {error}"))?
        }
    })
    .await
}

async fn build_ready_indexes<S, F, Fut>(ready: S, parallel: usize, build: F) -> eyre::Result<()>
where
    S: futures::Stream<Item = eyre::Result<DriveSyncInfo>>,
    F: Fn(DriveSyncInfo) -> Fut + Clone,
    Fut: std::future::Future<Output = eyre::Result<()>>,
{
    let first_error = Arc::new(std::sync::Mutex::new(None));
    let error_for_jobs = Arc::clone(&first_error);
    // Drain acquisition and await every started task even after the first error.
    ready
        .for_each_concurrent(parallel, move |item| {
            let build = build.clone();
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
                    build(info).await
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::DriveLetter;
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
                    execute_sync(work, parallel.into(), None, &scope).await?;
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
    #[tokio::test]
    async fn ready_indexes_respect_the_resolved_worker_limit() -> eyre::Result<()> {
        use std::sync::atomic::AtomicUsize;
        use std::sync::atomic::Ordering;

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
            build_ready_indexes(
                futures::stream::iter(infos.into_iter().map(Ok)),
                parallel,
                {
                    let active = Arc::clone(&active);
                    let peak = Arc::clone(&peak);
                    let completed = Arc::clone(&completed);
                    move |_| {
                        let active = Arc::clone(&active);
                        let peak = Arc::clone(&peak);
                        let completed = Arc::clone(&completed);
                        async move {
                            let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                            peak.fetch_max(current, Ordering::SeqCst);
                            tokio::task::yield_now().await;
                            active.fetch_sub(1, Ordering::SeqCst);
                            completed.fetch_add(1, Ordering::SeqCst);
                            Ok(())
                        }
                    }
                },
            )
            .await?;
            assert_eq!(peak.load(Ordering::SeqCst), parallel);
            assert_eq!(active.load(Ordering::SeqCst), 0);
            assert_eq!(completed.load(Ordering::SeqCst), 5);
        }
        Ok(())
    }
    #[tokio::test]
    async fn acquisition_failure_waits_for_another_active_index() -> eyre::Result<()> {
        use std::sync::atomic::AtomicBool;
        use std::sync::atomic::Ordering;
        let temp = tempfile::tempdir()?;
        let info = crate::sync::drive_sync_info::resolve_drive_infos_in_dir_for_letters(
            temp.path(),
            [DriveLetter::C],
        )?
        .remove(0);
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let channels = Arc::new(std::sync::Mutex::new(Some((started_tx, release_rx))));
        let finished = Arc::new(AtomicBool::new(false));
        let finished_job = Arc::clone(&finished);
        let job = tokio::spawn(async move {
            build_ready_indexes(
                futures::stream::iter([
                    Ok(info),
                    Err(eyre::eyre!("injected other-drive read failure")),
                ]),
                2,
                move |_| {
                    let (started, release) = channels.lock().unwrap().take().unwrap();
                    let finished = Arc::clone(&finished_job);
                    async move {
                        let _ = started.send(());
                        release.await?;
                        finished.store(true, Ordering::SeqCst);
                        Ok(())
                    }
                },
            )
            .await
        });
        started_rx.await?;
        tokio::task::yield_now().await;
        assert!(
            !job.is_finished(),
            "failure must not detach an already-started job"
        );
        release_tx.send(()).unwrap();
        assert!(
            job.await?
                .unwrap_err()
                .to_string()
                .contains("other-drive read failure")
        );
        assert!(finished.load(Ordering::SeqCst));
        Ok(())
    }
    #[tokio::test]
    async fn failure_drains_another_active_acquisition_before_returning() -> eyre::Result<()> {
        let temp = tempfile::tempdir()?;
        let info = crate::sync::drive_sync_info::resolve_drive_infos_in_dir_for_letters(
            temp.path(),
            [DriveLetter::G],
        )?
        .remove(0);
        let marker = temp.path().join("completed-acquisition");
        let marker_job = marker.clone();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let acquisition = futures::stream::once(async move {
            tokio::task::spawn_blocking(move || {
                let _ = started_tx.send(());
                release_rx.recv().unwrap();
                std::fs::write(marker_job, b"completed before return")?;
                eyre::Ok(info)
            })
            .await
            .map_err(|error| eyre::eyre!(error))?
        });
        let ready =
            futures::stream::iter([Err(eyre::eyre!("first drive failed"))]).chain(acquisition);
        let job = tokio::spawn(build_ready_indexes(ready, 2, |_| async {
            eyre::bail!("index must not start after read failure")
        }));
        started_rx.await?;
        tokio::task::yield_now().await;
        assert!(!job.is_finished());
        release_tx.send(())?;
        assert!(
            job.await?
                .unwrap_err()
                .to_string()
                .contains("first drive failed")
        );
        assert_eq!(std::fs::read(marker)?, b"completed before return");
        Ok(())
    }
}
