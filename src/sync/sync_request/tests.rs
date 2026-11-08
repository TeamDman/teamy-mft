use super::*;
use crate::cancellation::CancellationToken;
use crate::machine::config::load_checkpoint;
use crate::search_index::format::SearchIndexPathRow;
use std::collections::BTreeSet;
use std::sync::Condvar;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;

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
                let request = SyncRequest {
                    sync_dir: temp.path().to_path_buf(),
                    drive_letters: vec![DriveLetter::C],
                    if_exists: IfExistsOutputBehaviour::Skip,
                    parallel: 1.into(),
                    console_pid: None,
                };
                let work = request.prepare()?;
                let expected_decision = if index_exists {
                    SyncDecision::Skip
                } else {
                    SyncDecision::Index
                };
                assert_eq!(work[&DriveLetter::C].1, expected_decision);
                request.execute_drives(&work, &scope).await?;
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
                SyncRequest::publish_sidecars(&work, &cursors)?;
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
    let request = SyncRequest {
        sync_dir: temp.path().to_path_buf(),
        drive_letters: vec![DriveLetter::C, DriveLetter::D, DriveLetter::E],
        if_exists: IfExistsOutputBehaviour::Skip,
        parallel: 1.into(),
        console_pid: None,
    };
    let work = request.prepare()?;
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
    SyncRequest::publish_sidecars(&work, &cursors)?;
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
    for (mft_exists, index_exists) in [(false, false), (true, false), (false, true), (true, true)] {
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
            let request = SyncRequest {
                sync_dir: temp.path().to_path_buf(),
                drive_letters: vec![DriveLetter::C],
                if_exists: policy,
                parallel: 1.into(),
                console_pid: None,
            };
            let result = request.prepare();
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
    let cached_mft = DriveArtifactInfo::new(temp.path(), DriveLetter::C);
    let retained_index = DriveArtifactInfo::new(temp.path(), DriveLetter::D);
    let retained_snapshot = DriveArtifactInfo::new(temp.path(), DriveLetter::E);
    std::fs::write(&cached_mft.mft_path, b"cached MFT")?;
    std::fs::write(&retained_index.base_index_path, b"retained index")?;
    std::fs::write(&retained_snapshot.mft_path, b"retained MFT")?;
    std::fs::write(&retained_snapshot.base_index_path, b"retained index")?;

    let request = SyncRequest {
        sync_dir: temp.path().to_path_buf(),
        drive_letters: vec![
            DriveLetter::G,
            DriveLetter::E,
            DriveLetter::C,
            DriveLetter::D,
            DriveLetter::C,
        ],
        if_exists: IfExistsOutputBehaviour::Skip,
        parallel: 1.into(),
        console_pid: None,
    };
    let work = request.prepare()?;

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
    for (&drive, (info, _)) in &work {
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
                let request = SyncRequest {
                    sync_dir: temp.path().to_path_buf(),
                    drive_letters: vec![DriveLetter::C, DriveLetter::D, DriveLetter::G],
                    if_exists: IfExistsOutputBehaviour::Skip,
                    parallel: parallel.into(),
                    console_pid: None,
                };
                for &drive in &request.drive_letters {
                    let info = DriveArtifactInfo::new(temp.path(), drive);
                    std::fs::write(&info.mft_path, crate::mft::sample_mft_records())?;
                }
                let work = request.prepare()?;
                assert!(
                    work.values()
                        .all(|(_, decision)| *decision == SyncDecision::Index),
                    "fixture must never open a volume"
                );
                request.execute_drives(&work, &scope).await?;
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
    let dispatch = tracing::Dispatch::new(tracing_subscriber::registry().with(DaemonTraceLayer));
    let span = tracing::dispatcher::with_default(&dispatch, || {
        tracing::info_span!(
            "daemon_rpc",
            correlation_id = %correlation_id,
            rpc_method = "sync",
        )
    });
    SyncRequest::build_ready_indexes(
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
async fn ready_indexes_run_on_distinct_rayon_threads_with_the_resolved_limit() -> eyre::Result<()> {
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
        let job = tokio::spawn(SyncRequest::build_ready_indexes(
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
    let mut job = tokio::spawn(SyncRequest::build_ready_indexes(
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
async fn ready_index_overlaps_a_blocking_acquisition_on_a_single_thread_runtime() -> eyre::Result<()>
{
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
                        *acquisition_thread.0.lock().unwrap() = Some(std::thread::current().id());
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
    let mut job = tokio::spawn(SyncRequest::build_ready_indexes(
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
