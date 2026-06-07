use crate::machine::config::MachineConfig;
use crate::machine::config::PublishedCheckpoint;
use crate::machine::config::current_unix_ms;
use crate::machine::config::load_machine_config;
use crate::machine::config::published_drive_paths;
use crate::machine::config::save_checkpoint;
use crate::machine::daemon_log::daemon_log_hub;
use crate::machine::daemon_log::spawn_correlation_log_forwarder;
use crate::machine::daemon_log::stop_log_forwarder;
use crate::machine::ipc::CorrelationId;
use crate::machine::ipc::DegradedDriveStatus;
use crate::machine::ipc::LogStreamRequest;
use crate::machine::ipc::MachineDaemonRpc;
use crate::machine::ipc::MachineError;
use crate::machine::ipc::PingResponse;
use crate::machine::ipc::RpcQueryResponse;
use crate::machine::ipc::StatusRequest;
use crate::machine::ipc::StatusResponse;
use crate::machine::live_drive_state::LiveDriveState;
use crate::machine::usn::JournalCursor;
use crate::machine::usn::VolumeUsnJournalHandle;
use crate::query::QueryFilter;
use crate::query::QueryIgnoreRules;
use crate::query::QueryLimit;
use crate::query::QueryPlan;
use crate::query::QueryResultRow;
use crate::query::visit_drive_search_index_rows;
use crate::search_index::format::SEARCH_INDEX_VERSION;
use crate::sync::IfExistsOutputBehaviour;
use crate::sync::SyncPlan;
use crate::sync::execute_sync;
use crate::sync::resolve_drive_infos_in_dir_for_letters;
use crossbeam_channel::Receiver;
use crossbeam_channel::Sender;
use eyre::ContextCompat;
use rustc_hash::FxHashMap;
use std::ffi::c_void;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicIsize;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;
use tokio::net::windows::named_pipe::NamedPipeServer;
use tokio::net::windows::named_pipe::ServerOptions;
use tokio::sync::Mutex;
use tokio::sync::oneshot;
use tracing::Instrument;
use tracing::debug;
use tracing::info;
use tracing::warn;
use windows::Win32::Foundation::NO_ERROR;
use windows::Win32::System::Services::RegisterServiceCtrlHandlerExW;
use windows::Win32::System::Services::SERVICE_ACCEPT_SHUTDOWN;
use windows::Win32::System::Services::SERVICE_ACCEPT_STOP;
use windows::Win32::System::Services::SERVICE_CONTROL_SHUTDOWN;
use windows::Win32::System::Services::SERVICE_CONTROL_STOP;
use windows::Win32::System::Services::SERVICE_RUNNING;
use windows::Win32::System::Services::SERVICE_START_PENDING;
use windows::Win32::System::Services::SERVICE_STATUS;
use windows::Win32::System::Services::SERVICE_STATUS_CURRENT_STATE;
use windows::Win32::System::Services::SERVICE_STATUS_HANDLE;
use windows::Win32::System::Services::SERVICE_STOP_PENDING;
use windows::Win32::System::Services::SERVICE_STOPPED;
use windows::Win32::System::Services::SERVICE_TABLE_ENTRYW;
use windows::Win32::System::Services::SERVICE_WIN32_OWN_PROCESS;
use windows::Win32::System::Services::SetServiceStatus;
use windows::Win32::System::Services::StartServiceCtrlDispatcherW;
use windows::core::PCWSTR;

static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);
static SERVICE_STATUS_HANDLE_SLOT: AtomicIsize = AtomicIsize::new(0);
static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

type DaemonPipeReader = Box<dyn tokio::io::AsyncRead + Send + Unpin>;
type DaemonPipeWriter = Box<dyn tokio::io::AsyncWrite + Send + Unpin>;
type DaemonPipeLink = vox_stream::StreamLink<DaemonPipeReader, DaemonPipeWriter>;

struct MachineDaemonPipeAcceptor {
    addr: String,
    owner_sid: String,
    pending: Mutex<NamedPipeServer>,
}

#[derive(Debug, Clone)]
pub struct MachineCacheSyncResult {
    pub synced_drives: Vec<char>,
    pub live_drives: Vec<char>,
    pub skipped_drives: Vec<(char, String)>,
}

type SupportedDriveSyncOutcome = (
    Vec<char>,
    FxHashMap<char, JournalCursor>,
    Vec<(char, String)>,
);

#[derive(Debug)]
struct DaemonRuntimeState {
    owner_sid: String,
    sync_dir: std::path::PathBuf,
    drives: FxHashMap<char, LiveDriveState>,
    degraded: FxHashMap<char, String>,
    loading: FxHashMap<char, String>,
    active_jobs: usize,
    warm_drive_letters: Vec<char>,
    next_warm_drive_index: usize,
    warm_not_before: Instant,
}

#[derive(Debug, Clone)]
struct MachineDaemonService {
    config: MachineConfig,
    worker: DaemonWorker,
}

#[derive(Debug)]
struct DaemonDriveQueryRows {
    rows: Vec<crate::query::QueryResultRow>,
    degraded: Option<(char, String)>,
}

#[derive(Debug)]
struct DaemonQueryOutcome {
    rows: Vec<crate::query::QueryResultRow>,
}

enum DriveWorkerCommand {
    Query {
        request: QueryPlan,
        correlation_id: CorrelationId,
        rpc_method: &'static str,
        cancel: Arc<AtomicBool>,
        response: oneshot::Sender<Result<DaemonDriveQueryRows, MachineError>>,
    },
    Warm,
    Flush,
    Stop,
}

#[derive(Debug, Clone, Default)]
struct DriveWorkerStatusSnapshot {
    loaded: bool,
    loading: bool,
    snapshot_only: Option<String>,
    degraded: Option<String>,
    active_jobs: usize,
}

#[derive(Clone)]
struct DriveWorker {
    drive: char,
    tx: Sender<DriveWorkerCommand>,
    status: Arc<StdMutex<DriveWorkerStatusSnapshot>>,
    stop_requested: Arc<AtomicBool>,
}

enum DaemonWorkerCommand {
    Sync {
        request: SyncPlan,
        correlation_id: CorrelationId,
        response: oneshot::Sender<Result<(), MachineError>>,
    },
    Flush,
    Stop,
}

#[derive(Clone)]
struct DaemonWorker {
    tx: Sender<DaemonWorkerCommand>,
    status: Arc<StdMutex<DaemonWorkerStatusSnapshot>>,
    drive_workers: Arc<StdMutex<FxHashMap<char, DriveWorker>>>,
    sync_dir: std::path::PathBuf,
    stop_requested: Arc<AtomicBool>,
}

#[derive(Debug, Clone, Default)]
struct DaemonWorkerStatusSnapshot {
    loaded_drive_letters: Vec<char>,
    loading_drive_letters: Vec<char>,
    snapshot_only_drive_letters: Vec<char>,
    degraded_drives: Vec<DegradedDriveStatus>,
    active_job_count: usize,
}

impl std::fmt::Debug for DaemonWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DaemonWorker").finish_non_exhaustive()
    }
}

impl std::fmt::Debug for DriveWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DriveWorker")
            .field("drive", &self.drive)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
struct DriveWorkerState {
    drive: char,
    sync_dir: std::path::PathBuf,
    state: Option<LiveDriveState>,
    snapshot_only: Option<String>,
    degraded: Option<String>,
    loading: bool,
    active_jobs: usize,
}

impl DriveWorker {
    fn start(drive: char, sync_dir: std::path::PathBuf) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        let status = Arc::new(StdMutex::new(DriveWorkerStatusSnapshot::default()));
        let stop_requested = Arc::new(AtomicBool::new(false));
        std::thread::Builder::new()
            .name(format!("teamy-mft-drive-{drive}"))
            .spawn({
                let status_for_thread = Arc::clone(&status);
                let stop_requested_for_thread = Arc::clone(&stop_requested);
                move || {
                    let mut state = DriveWorkerState {
                        drive,
                        sync_dir,
                        state: None,
                        snapshot_only: None,
                        degraded: None,
                        loading: false,
                        active_jobs: 0,
                    };
                    run_drive_worker(
                        &mut state,
                        &rx,
                        &status_for_thread,
                        &stop_requested_for_thread,
                    );
                }
            })
            .expect("failed to spawn daemon drive worker thread");
        Self {
            drive,
            tx,
            status,
            stop_requested,
        }
    }

    async fn query(
        &self,
        request: QueryPlan,
        correlation_id: CorrelationId,
        rpc_method: &'static str,
        cancel: Arc<AtomicBool>,
    ) -> Result<DaemonDriveQueryRows, MachineError> {
        let (response, rx) = oneshot::channel();
        self.tx
            .send(DriveWorkerCommand::Query {
                request,
                correlation_id,
                rpc_method,
                cancel,
                response,
            })
            .map_err(|_send_error| MachineError::degraded("daemon drive worker stopped"))?;
        rx.await
            .map_err(|_recv_error| MachineError::degraded("daemon drive worker stopped"))?
    }

    fn snapshot(&self) -> DriveWorkerStatusSnapshot {
        self.status
            .lock()
            .map(|snapshot| snapshot.clone())
            .unwrap_or_default()
    }

    fn flush(&self) {
        let _ = self.tx.send(DriveWorkerCommand::Flush);
    }

    fn warm(&self) {
        let _ = self.tx.send(DriveWorkerCommand::Warm);
    }

    fn stop(&self) {
        self.stop_requested.store(true, Ordering::Relaxed);
        let _ = self.tx.send(DriveWorkerCommand::Stop);
    }
}

impl DriveWorkerState {
    fn query_with_cancel(
        &mut self,
        request: &QueryPlan,
        cancel: &AtomicBool,
    ) -> Result<DaemonDriveQueryRows, MachineError> {
        if cancel.load(Ordering::Relaxed) {
            return Ok(DaemonDriveQueryRows {
                rows: Vec::new(),
                degraded: None,
            });
        }
        if let Err(error) = self.refresh_with_cancel(Some(cancel)) {
            if cancel.load(Ordering::Relaxed) {
                return Ok(DaemonDriveQueryRows {
                    rows: Vec::new(),
                    degraded: None,
                });
            }
            return match query_published_drive(self.drive, &self.sync_dir, request) {
                Ok(rows) => {
                    let message = format!(
                        "{}; served published cache for this drive instead",
                        error.message
                    );
                    self.mark_snapshot_only(message.clone());
                    Ok(DaemonDriveQueryRows {
                        rows,
                        degraded: Some((self.drive, message)),
                    })
                }
                Err(fallback_error) => Err(MachineError::degraded(format!(
                    "{}; published cache fallback also failed: {fallback_error}",
                    error.message
                ))),
            };
        }

        self.state
            .as_mut()
            .ok_or_else(|| MachineError::degraded(format!("Drive {} is not loaded", self.drive)))?
            .query_with_cancel(request, Some(cancel))
            .map(|rows| DaemonDriveQueryRows {
                rows,
                degraded: None,
            })
    }

    fn refresh(&mut self) -> Result<(), MachineError> {
        self.refresh_with_cancel(None)
    }

    fn refresh_with_cancel(&mut self, cancel: Option<&AtomicBool>) -> Result<(), MachineError> {
        if let Some(message) = self.degraded.clone() {
            return Err(MachineError::degraded(message));
        }

        if self.state.is_none() {
            self.loading = true;
            let paths = published_drive_paths(&self.sync_dir, self.drive);
            let state = (|| -> eyre::Result<LiveDriveState> {
                if !paths.mft_path.is_file() {
                    eyre::bail!(
                        "Drive {} has no published MFT snapshot at {}",
                        self.drive,
                        paths.mft_path.display()
                    );
                }
                if !paths.base_index_path.is_file() {
                    eyre::bail!(
                        "Drive {} has no published base index at {}",
                        self.drive,
                        paths.base_index_path.display()
                    );
                }
                LiveDriveState::load_with_cancel(&self.sync_dir, paths, cancel)
            })()
            .map_err(|error| {
                let message = format!(
                    "Drive {} could not be loaded for live query: {error}",
                    self.drive
                );
                self.loading = false;
                self.degraded = Some(message.clone());
                MachineError::degraded(message)
            })?;
            self.loading = false;
            self.snapshot_only = None;
            self.state = Some(state);
        }

        let refresh_result = self
            .state
            .as_mut()
            .expect("drive should be loaded before refresh")
            .refresh_with_cancel(cancel);
        if let Err(error) = refresh_result {
            self.state = None;
            let message = format!(
                "Drive {} could not be refreshed for live query: {error}",
                self.drive
            );
            self.degraded = Some(message.clone());
            return Err(MachineError::degraded(message));
        }
        Ok(())
    }

    fn flush(&mut self) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        if !state.published_dirty() {
            return;
        }
        if let Err(error) = state.flush_published() {
            warn!(drive = %self.drive, error = %error, "Failed flushing live overlay during daemon shutdown/idle");
        }
    }

    fn mark_snapshot_only(&mut self, message: String) {
        self.snapshot_only = Some(message);
        self.degraded = None;
        self.loading = false;
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "drive worker command handling is intentionally kept in one loop so state transitions stay visible"
)]
fn run_drive_worker(
    state: &mut DriveWorkerState,
    rx: &Receiver<DriveWorkerCommand>,
    status: &StdMutex<DriveWorkerStatusSnapshot>,
    stop_requested: &AtomicBool,
) {
    publish_drive_worker_status(state, status);
    loop {
        if stop_requested.load(Ordering::Relaxed) || STOP_REQUESTED.load(Ordering::Relaxed) {
            state.flush();
            publish_drive_worker_status(state, status);
            break;
        }
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(DriveWorkerCommand::Query {
                request,
                correlation_id,
                rpc_method,
                cancel,
                response,
            }) => {
                state.active_jobs += 1;
                state.loading = state.state.is_none() && state.degraded.is_none();
                publish_drive_worker_status(state, status);
                let span = tracing::info_span!(
                    "daemon_rpc",
                    correlation_id = %correlation_id,
                    rpc_method
                );
                let result = {
                    let _entered = span.enter();
                    if cancel.load(Ordering::Relaxed) {
                        Ok(DaemonDriveQueryRows {
                            rows: Vec::new(),
                            degraded: None,
                        })
                    } else {
                        std::panic::catch_unwind(AssertUnwindSafe(|| {
                            state.query_with_cancel(&request, &cancel)
                        }))
                        .map_err(|payload| {
                            machine_error_from_panic("query request panicked", payload)
                        })
                        .and_then(|result| result)
                    }
                };
                state.active_jobs = state.active_jobs.saturating_sub(1);
                state.loading = false;
                publish_drive_worker_status(state, status);
                let _ = response.send(result);
            }
            Ok(DriveWorkerCommand::Flush) => {
                state.flush();
                publish_drive_worker_status(state, status);
            }
            Ok(DriveWorkerCommand::Warm) => {
                if state.state.is_none() && state.degraded.is_none() {
                    state.loading = true;
                    publish_drive_worker_status(state, status);
                }
                if let Err(error) = state.refresh() {
                    match published_drive_cache_available(state.drive, &state.sync_dir) {
                        Ok(true) => {
                            let message = format!(
                                "{}; served published cache for this drive instead",
                                error.message
                            );
                            state.mark_snapshot_only(message.clone());
                            warn!(
                                drive = %state.drive,
                                error = %message,
                                "Drive warmup fell back to published snapshot"
                            );
                        }
                        Ok(false) => {
                            warn!(
                                drive = %state.drive,
                                error = %error.message,
                                "Drive warmup failed and no published snapshot fallback is available"
                            );
                        }
                        Err(fallback_error) => {
                            warn!(
                                drive = %state.drive,
                                error = %error.message,
                                fallback_error = %fallback_error,
                                "Drive warmup failed while checking published snapshot fallback"
                            );
                        }
                    }
                }
                state.loading = false;
                publish_drive_worker_status(state, status);
            }
            Ok(DriveWorkerCommand::Stop)
            | Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                state.flush();
                publish_drive_worker_status(state, status);
                break;
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                if state.state.is_none() || state.degraded.is_some() {
                    publish_drive_worker_status(state, status);
                    continue;
                }
                if let Err(error) = state.refresh() {
                    warn!(
                        drive = %state.drive,
                        error = %error.message,
                        "Drive refresh degraded; falling back to disk until next reload"
                    );
                }
                publish_drive_worker_status(state, status);
            }
        }
    }
}

fn publish_drive_worker_status(
    state: &DriveWorkerState,
    status: &StdMutex<DriveWorkerStatusSnapshot>,
) {
    if let Ok(mut snapshot) = status.lock() {
        *snapshot = DriveWorkerStatusSnapshot {
            loaded: state.state.is_some(),
            loading: state.loading,
            snapshot_only: state.snapshot_only.clone(),
            degraded: state.degraded.clone(),
            active_jobs: state.active_jobs,
        };
    }
}

impl DaemonWorker {
    fn start(config: &MachineConfig) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut state = DaemonRuntimeState::new(config);
        let status = Arc::new(StdMutex::new(DaemonWorkerStatusSnapshot::default()));
        let drive_workers = Arc::new(StdMutex::new(FxHashMap::default()));
        let stop_requested = Arc::new(AtomicBool::new(false));
        std::thread::Builder::new()
            .name("teamy-mft-daemon-worker".to_owned())
            .spawn({
                let status_for_thread = Arc::clone(&status);
                let drive_workers_for_thread = Arc::clone(&drive_workers);
                let stop_requested_for_thread = Arc::clone(&stop_requested);
                move || {
                    run_daemon_worker(
                        &mut state,
                        &rx,
                        &status_for_thread,
                        &drive_workers_for_thread,
                        &stop_requested_for_thread,
                    );
                }
            })
            .expect("failed to spawn daemon worker thread");
        Self {
            tx,
            status,
            drive_workers,
            sync_dir: config.sync_dir.clone().into_inner(),
            stop_requested,
        }
    }

    async fn query(
        &self,
        request: QueryPlan,
        correlation_id: CorrelationId,
        rpc_method: &'static str,
        cancel: Arc<AtomicBool>,
    ) -> Result<DaemonQueryOutcome, MachineError> {
        self.query_drive_workers(request, correlation_id, rpc_method, cancel)
            .await
    }

    async fn sync(
        &self,
        request: SyncPlan,
        correlation_id: CorrelationId,
    ) -> Result<(), MachineError> {
        let (response, rx) = oneshot::channel();
        self.tx
            .send(DaemonWorkerCommand::Sync {
                request,
                correlation_id,
                response,
            })
            .map_err(|_send_error| MachineError::degraded("daemon worker stopped"))?;
        rx.await
            .map_err(|_recv_error| MachineError::degraded("daemon worker stopped"))?
    }

    fn snapshot(&self) -> DaemonWorkerStatusSnapshot {
        let mut snapshot = self
            .status
            .lock()
            .map(|snapshot| snapshot.clone())
            .unwrap_or_default();
        if let Ok(workers) = self.drive_workers.lock() {
            for (&drive, worker) in workers.iter() {
                let drive_status = worker.snapshot();
                if drive_status.loaded {
                    snapshot.loaded_drive_letters.push(drive);
                }
                if drive_status.loading {
                    snapshot.loading_drive_letters.push(drive);
                }
                if drive_status.snapshot_only.is_some() {
                    snapshot.snapshot_only_drive_letters.push(drive);
                }
                if let Some(message) = drive_status.degraded {
                    snapshot.degraded_drives.push(DegradedDriveStatus {
                        drive_letter: drive,
                        message,
                    });
                }
                snapshot.active_job_count += drive_status.active_jobs;
            }
        }
        snapshot.loaded_drive_letters.sort_unstable();
        snapshot.loaded_drive_letters.dedup();
        snapshot.loading_drive_letters.sort_unstable();
        snapshot.loading_drive_letters.dedup();
        snapshot.snapshot_only_drive_letters.sort_unstable();
        snapshot.snapshot_only_drive_letters.dedup();
        snapshot
    }

    fn flush(&self) {
        let _ = self.tx.send(DaemonWorkerCommand::Flush);
        if let Ok(workers) = self.drive_workers.lock() {
            for worker in workers.values() {
                worker.flush();
            }
        }
    }

    fn stop(&self) {
        self.stop_requested.store(true, Ordering::Relaxed);
        let _ = self.tx.send(DaemonWorkerCommand::Stop);
        if let Ok(workers) = self.drive_workers.lock() {
            for worker in workers.values() {
                worker.stop();
            }
        }
    }
}

impl DaemonWorker {
    async fn query_drive_workers(
        &self,
        request: QueryPlan,
        correlation_id: CorrelationId,
        rpc_method: &'static str,
        cancel: Arc<AtomicBool>,
    ) -> Result<DaemonQueryOutcome, MachineError> {
        let mut rows = Vec::new();
        let mut queried_drives = 0usize;
        let mut degraded_drives = Vec::new();
        let drive_letters = request
            .drive_letter_pattern
            .clone()
            .into_drive_letters()
            .map_err(|error| MachineError::request_invalid(error.to_string()))?;

        for &drive in &drive_letters {
            if cancel.load(Ordering::Relaxed) {
                tracing::warn!("Daemon query cancelled by client");
                break;
            }
            let mut per_drive_request = request.clone();
            if let Some(limit) = request.limit.get() {
                let Some(remaining) = limit.checked_sub(rows.len()) else {
                    break;
                };
                if remaining == 0 {
                    break;
                }
                per_drive_request.limit = QueryLimit::from(remaining);
            }

            match self
                .drive_worker(drive)
                .query(
                    per_drive_request,
                    correlation_id.clone(),
                    rpc_method,
                    Arc::clone(&cancel),
                )
                .await
            {
                Ok(drive_rows) => {
                    if let Some(degraded) = drive_rows.degraded {
                        degraded_drives.push(degraded);
                    }
                    queried_drives += 1;
                    rows.extend(drive_rows.rows);
                }
                Err(error) => degraded_drives.push((drive, error.message)),
            }
        }

        if queried_drives == 0 && !degraded_drives.is_empty() {
            return Err(MachineError::degraded(format_degraded_query_drives(
                &degraded_drives,
            )));
        }

        if !degraded_drives.is_empty() {
            warn!(
                queried_drives,
                degraded_drive_count = degraded_drives.len(),
                degraded_drives = %format_degraded_query_drives(&degraded_drives),
                "Daemon query used published snapshot fallback for some drives"
            );
        }

        Ok(DaemonQueryOutcome { rows })
    }

    fn drive_worker(&self, drive: char) -> DriveWorker {
        let mut workers = self
            .drive_workers
            .lock()
            .expect("drive worker registry poisoned");
        workers
            .entry(drive)
            .or_insert_with(|| DriveWorker::start(drive, self.sync_dir.clone()))
            .clone()
    }
}

fn run_daemon_worker(
    state: &mut DaemonRuntimeState,
    rx: &Receiver<DaemonWorkerCommand>,
    status: &StdMutex<DaemonWorkerStatusSnapshot>,
    drive_workers: &StdMutex<FxHashMap<char, DriveWorker>>,
    stop_requested: &AtomicBool,
) {
    publish_worker_status(state, status);
    loop {
        if stop_requested.load(Ordering::Relaxed) || STOP_REQUESTED.load(Ordering::Relaxed) {
            state.flush_dirty_drives();
            publish_worker_status(state, status);
            break;
        }
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(DaemonWorkerCommand::Sync {
                request,
                correlation_id,
                response,
            }) => {
                state.active_jobs += 1;
                publish_worker_status(state, status);
                let drive_letters = match request.drive_letter_pattern.clone().into_drive_letters()
                {
                    Ok(drive_letters) => drive_letters,
                    Err(error) => {
                        state.active_jobs = state.active_jobs.saturating_sub(1);
                        publish_worker_status(state, status);
                        let _ = response.send(Err(MachineError::degraded(error.to_string())));
                        continue;
                    }
                };
                if let Ok(mut workers) = drive_workers.lock() {
                    for drive in &drive_letters {
                        if let Some(worker) = workers.remove(drive) {
                            worker.stop();
                        }
                    }
                }
                let span = tracing::info_span!(
                    "daemon_rpc",
                    correlation_id = %correlation_id,
                    rpc_method = "sync"
                );
                let result = {
                    let _entered = span.enter();
                    run_daemon_worker_sync(state, request)
                };
                state.active_jobs = state.active_jobs.saturating_sub(1);
                publish_worker_status(state, status);
                let _ = response.send(result);
            }
            Ok(DaemonWorkerCommand::Flush) => {
                state.flush_dirty_drives();
                if let Ok(workers) = drive_workers.lock() {
                    for worker in workers.values() {
                        worker.flush();
                    }
                }
                publish_worker_status(state, status);
            }
            Ok(DaemonWorkerCommand::Stop)
            | Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                state.flush_dirty_drives();
                if let Ok(workers) = drive_workers.lock() {
                    for worker in workers.values() {
                        worker.stop();
                    }
                }
                publish_worker_status(state, status);
                break;
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                if stop_requested.load(Ordering::Relaxed) || STOP_REQUESTED.load(Ordering::Relaxed)
                {
                    state.flush_dirty_drives();
                    publish_worker_status(state, status);
                    break;
                }
                state.refresh_loaded_drives();
                warm_next_drive_worker(state, drive_workers);
                publish_worker_status(state, status);
            }
        }
    }
}

fn warm_next_drive_worker(
    state: &mut DaemonRuntimeState,
    drive_workers: &StdMutex<FxHashMap<char, DriveWorker>>,
) {
    if state.warm_drive_letters.is_empty() {
        return;
    }
    if Instant::now() < state.warm_not_before {
        return;
    }

    for _ in 0..state.warm_drive_letters.len() {
        let index = state.next_warm_drive_index % state.warm_drive_letters.len();
        state.next_warm_drive_index = state.next_warm_drive_index.wrapping_add(1);
        let drive = state.warm_drive_letters[index];
        let Ok(mut workers) = drive_workers.lock() else {
            return;
        };
        if workers.values().any(|worker| {
            let snapshot = worker.snapshot();
            snapshot.loading || snapshot.active_jobs > 0
        }) {
            return;
        }
        let worker = workers
            .entry(drive)
            .or_insert_with(|| DriveWorker::start(drive, state.sync_dir.clone()))
            .clone();
        drop(workers);

        let snapshot = worker.snapshot();
        if snapshot.loading || snapshot.active_jobs > 0 || snapshot.degraded.is_some() {
            continue;
        }
        worker.warm();
        break;
    }
}

fn publish_worker_status(
    state: &DaemonRuntimeState,
    status: &StdMutex<DaemonWorkerStatusSnapshot>,
) {
    if let Ok(mut snapshot) = status.lock() {
        *snapshot = DaemonWorkerStatusSnapshot {
            loaded_drive_letters: state.drives.keys().copied().collect(),
            loading_drive_letters: state.loading.keys().copied().collect(),
            snapshot_only_drive_letters: Vec::new(),
            degraded_drives: state
                .degraded
                .iter()
                .map(|(&drive_letter, message)| DegradedDriveStatus {
                    drive_letter,
                    message: message.clone(),
                })
                .collect(),
            active_job_count: state.active_jobs,
        };
    }
}

fn run_daemon_worker_sync(
    state: &mut DaemonRuntimeState,
    request: SyncPlan,
) -> Result<(), MachineError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| MachineError::degraded(error.to_string()))?;
    runtime.block_on(state.sync(request))
}

fn query_published_drive(
    drive: char,
    sync_dir: &std::path::Path,
    request: &QueryPlan,
) -> eyre::Result<Vec<QueryResultRow>> {
    let query_plan = request.clone();
    let ignore_rules = match QueryIgnoreRules::discover_for_drive_letters(
        &[drive],
        sync_dir,
        request.profile.as_deref(),
    ) {
        Ok(rules) => Some(rules),
        Err(error) => {
            warn!(
                drive = %drive,
                error = %error,
                "Published-cache query could not load ignore rules; treating paths as visible"
            );
            None
        }
    };
    let filter = QueryFilter::new(request, ignore_rules)?;
    let limit = request.limit.get();
    let mut rows = Vec::with_capacity(limit.unwrap_or_default());

    visit_drive_search_index_rows(
        drive,
        sync_dir,
        &query_plan,
        request.include_deleted,
        request.only_deleted,
        |row| {
            if let Some(row) = filter.classify_and_match(row) {
                rows.push(row);
            }
            Ok(limit.is_none_or(|limit| rows.len() < limit))
        },
    )?;

    Ok(rows)
}

fn published_drive_cache_available(drive: char, sync_dir: &std::path::Path) -> eyre::Result<bool> {
    let paths = published_drive_paths(sync_dir, drive);
    Ok(std::fs::metadata(&paths.mft_path)
        .map(|metadata| metadata.is_file())
        .or_else(|error| match error.kind() {
            std::io::ErrorKind::NotFound => Ok(false),
            _ => Err(error),
        })?
        && std::fs::metadata(&paths.base_index_path)
            .map(|metadata| metadata.is_file())
            .or_else(|error| match error.kind() {
                std::io::ErrorKind::NotFound => Ok(false),
                _ => Err(error),
            })?)
}

fn collect_warm_drive_letters(sync_dir: &std::path::Path) -> Vec<char> {
    crate::machine::status::collect_published_drive_summaries(
        sync_dir,
        &crate::windows_utils::storage::DriveLetterPattern("*".to_owned()),
    )
    .map(|summaries| {
        summaries
            .into_iter()
            .filter(|summary| {
                summary.mft_path.is_file()
                    && summary.base_index_path.is_file()
                    && summary.warning.is_none()
            })
            .map(|summary| summary.drive_letter)
            .collect()
    })
    .unwrap_or_default()
}

impl DaemonRuntimeState {
    fn new(config: &MachineConfig) -> Self {
        Self {
            owner_sid: config.owner_sid.clone(),
            sync_dir: config.sync_dir.clone().into_inner(),
            drives: FxHashMap::default(),
            degraded: FxHashMap::default(),
            loading: FxHashMap::default(),
            active_jobs: 0,
            warm_drive_letters: collect_warm_drive_letters(config.sync_dir.as_path()),
            next_warm_drive_index: 0,
            warm_not_before: Instant::now() + Duration::from_secs(10),
        }
    }

    async fn sync(&mut self, request: SyncPlan) -> Result<(), MachineError> {
        let drive_letters = request
            .drive_letter_pattern
            .clone()
            .into_drive_letters()
            .map_err(|error| MachineError::degraded(error.to_string()))?;
        self.flush_dirty_drives();
        info!(
            drives = ?drive_letters,
            if_exists = ?request.if_exists,
            "daemon sync request starting"
        );
        crate::machine::security::restrict_path_to_owner(&self.sync_dir, &self.owner_sid)
            .map_err(|error| MachineError::degraded(error.to_string()))?;
        repair_published_drive_permissions(&self.sync_dir, &self.owner_sid, &drive_letters)
            .map_err(|error| MachineError::degraded(error.to_string()))?;
        let sync_result =
            sync_machine_cache_async(&self.sync_dir, &drive_letters, request.if_exists)
                .await
                .map_err(|error| MachineError::degraded(error.to_string()))?;

        debug!(
            synced_drives = ?sync_result.synced_drives,
            live_drives = ?sync_result.live_drives,
            skipped_drives = ?sync_result.skipped_drives,
            "Machine-managed sync completed"
        );

        for &drive in &drive_letters {
            self.drives.remove(&drive);
            self.degraded.remove(&drive);
        }
        for &drive in &sync_result.live_drives {
            self.refresh_drive(drive)?;
            self.drive_mut(drive)?
                .flush_published()
                .map_err(|error| MachineError::degraded(error.to_string()))?;
        }

        Ok(())
    }

    fn refresh_loaded_drives(&mut self) {
        let drives = self.drives.keys().copied().collect::<Vec<_>>();
        for drive in drives {
            if let Err(error) = self.refresh_drive(drive) {
                warn!(drive = %drive, error = %error.message, "Drive refresh degraded; falling back to disk until next reload");
            }
        }
    }

    fn flush_dirty_drives(&mut self) {
        for (&drive, state) in &mut self.drives {
            if !state.published_dirty() {
                continue;
            }
            if let Err(error) = state.flush_published() {
                warn!(drive = %drive, error = %error, "Failed flushing live overlay during daemon shutdown/idle");
            }
        }
    }

    fn refresh_drive(&mut self, drive: char) -> Result<(), MachineError> {
        if let Some(message) = self.degraded.get(&drive).cloned() {
            return Err(MachineError::degraded(message));
        }

        if !self.drives.contains_key(&drive) {
            self.loading.insert(drive, "loading".to_owned());
            let state = self.load_drive_state(drive).map_err(|error| {
                let message = format!("Drive {drive} could not be loaded for live query: {error}");
                self.loading.remove(&drive);
                self.degraded.insert(drive, message.clone());
                MachineError::degraded(message)
            })?;
            self.loading.remove(&drive);
            self.drives.insert(drive, state);
        }

        let refresh_result = self
            .drives
            .get_mut(&drive)
            .expect("drive should be loaded before refresh")
            .refresh();
        if let Err(error) = refresh_result {
            self.drives.remove(&drive);
            let message = error.to_string();
            let message = format!("Drive {drive} could not be refreshed for live query: {message}");
            self.degraded.insert(drive, message.clone());
            return Err(MachineError::degraded(message));
        }
        Ok(())
    }

    fn drive_mut(&mut self, drive: char) -> Result<&mut LiveDriveState, MachineError> {
        self.drives
            .get_mut(&drive)
            .ok_or_else(|| MachineError::degraded(format!("Drive {drive} is not loaded")))
    }

    fn load_drive_state(&self, drive: char) -> eyre::Result<LiveDriveState> {
        let paths = published_drive_paths(&self.sync_dir, drive);
        if !paths.mft_path.is_file() {
            eyre::bail!(
                "Drive {} has no published MFT snapshot at {}",
                drive,
                paths.mft_path.display()
            );
        }
        if !paths.base_index_path.is_file() {
            eyre::bail!(
                "Drive {} has no published base index at {}",
                drive,
                paths.base_index_path.display()
            );
        }
        LiveDriveState::load(&self.sync_dir, paths)
    }
}

fn format_degraded_query_drives(degraded_drives: &[(char, String)]) -> String {
    degraded_drives
        .iter()
        .map(|(drive, message)| format!("{drive}: {message}"))
        .collect::<Vec<_>>()
        .join("; ")
}

impl MachineDaemonService {
    fn new(config: MachineConfig) -> Self {
        let worker = DaemonWorker::start(&config);
        Self { config, worker }
    }

    async fn run_query_in_span(
        &self,
        request: QueryPlan,
        correlation_id: &CorrelationId,
    ) -> Result<Vec<crate::query::QueryResultRow>, MachineError> {
        let worker = self.worker.clone();
        let request_for_body = request.clone();
        let span = tracing::info_span!(
            "daemon_rpc",
            correlation_id = %correlation_id,
            rpc_method = "query"
        );
        async move {
            tracing::info!(
                query_groups = request_for_body.query.groups().len(),
                drive_pattern = %request_for_body.drive_letter_pattern,
                limit = ?request_for_body.limit,
                "Running daemon query"
            );
            match worker
                .query(
                    request_for_body,
                    correlation_id.clone(),
                    "query",
                    Arc::new(AtomicBool::new(false)),
                )
                .await
            {
                Ok(outcome) => {
                    tracing::info!(matched_rows = outcome.rows.len(), "Daemon query completed");
                    Ok(outcome.rows)
                }
                Err(error) => {
                    tracing::warn!(error = %error.message, "Daemon query degraded");
                    Err(error)
                }
            }
        }
        .instrument(span)
        .await
    }

    async fn run_query_stream_in_span(
        &self,
        request: QueryPlan,
        rows: &vox::Tx<QueryResultRow>,
        cancel: &mut vox::Rx<u8>,
        correlation_id: &CorrelationId,
    ) -> Result<(), MachineError> {
        let worker = self.worker.clone();
        let request_for_body = request.clone();
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let cancel_flag_for_watcher = Arc::clone(&cancel_flag);
        let span = tracing::info_span!(
            "daemon_rpc",
            correlation_id = %correlation_id,
            rpc_method = "query_stream"
        );
        async move {
            tracing::info!(
                query_groups = request_for_body.query.groups().len(),
                drive_pattern = %request_for_body.drive_letter_pattern,
                limit = ?request_for_body.limit,
                "Running daemon streamed query"
            );
            let mut emitted_rows = 0usize;
            let query = worker.query(
                request_for_body.clone(),
                correlation_id.clone(),
                "query_stream",
                Arc::clone(&cancel_flag),
            );
            tokio::pin!(query);
            let outcome = loop {
                tokio::select! {
                    response = &mut query => break response?,
                    cancel_result = cancel.recv() => {
                        match cancel_result {
                            Ok(Some(_) | None) => {
                                cancel_flag_for_watcher.store(true, Ordering::Relaxed);
                                tracing::warn!("Daemon query stream cancelled by client");
                                return Ok(());
                            }
                            Err(error) => {
                                tracing::debug!(error = %error, "Daemon query cancel channel failed");
                            }
                        }
                    }
                }
            };
            for row in outcome.rows {
                if cancel_flag.load(Ordering::Relaxed) || query_stream_cancelled(cancel).await {
                    tracing::warn!("Daemon query stream cancelled by client");
                    return Ok(());
                }
                if request_for_body
                    .limit
                    .is_some_and(|limit| emitted_rows >= limit)
                {
                    break;
                }
                if rows.send(row).await.is_err() {
                    return Ok(());
                }
                emitted_rows += 1;
            }
            tracing::info!(matched_rows = emitted_rows, "Daemon query stream completed");
            Ok(())
        }
        .instrument(span)
        .await
    }
}

async fn query_stream_cancelled(cancel: &mut vox::Rx<u8>) -> bool {
    tokio::select! {
        cancel_result = cancel.recv() => {
            match cancel_result {
                Ok(Some(_) | None) => true,
                Err(error) => {
                    tracing::debug!(error = %error, "Daemon query cancel channel failed");
                    false
                }
            }
        }
        () = tokio::time::sleep(Duration::ZERO) => false,
    }
}

fn next_correlation_id(method: &str) -> CorrelationId {
    let _ = method;
    let _ = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    CorrelationId::new()
}

fn repair_published_drive_permissions(
    sync_dir: &std::path::Path,
    owner_sid: &str,
    drive_letters: &[char],
) -> eyre::Result<()> {
    for &drive in drive_letters {
        let paths = published_drive_paths(sync_dir, drive);
        for artifact_path in [
            &paths.mft_path,
            &paths.base_index_path,
            &paths.overlay_index_path,
            &paths.checkpoint_path,
        ] {
            if !artifact_path.exists() {
                continue;
            }
            crate::machine::security::restrict_path_to_owner(artifact_path, owner_sid)?;
        }
    }
    Ok(())
}

impl MachineDaemonRpc for MachineDaemonService {
    async fn ping(
        &self,
        logs: vox::Tx<crate::machine::daemon_log::DaemonLogWireEvent>,
    ) -> Result<PingResponse, MachineError> {
        let correlation_id = next_correlation_id("ping");
        let log_forwarder = spawn_correlation_log_forwarder(correlation_id.clone(), logs);
        let service_name = self.config.service_name.clone();
        let span = tracing::info_span!(
            "daemon_rpc",
            correlation_id = %correlation_id,
            rpc_method = "ping"
        );
        let response = async move {
            tracing::info!(service_name = %service_name, "Daemon pong");
            Ok(PingResponse {
                service_name,
                build: crate::machine::ipc::DaemonBuildInfo {
                    app_version: String::from(crate::APP_SEMVER),
                    git_revision: String::from(crate::APP_GIT_REVISION),
                    build_unix_ms: crate::APP_BUILD_UNIX_MS.parse().unwrap_or(0),
                    rpc_compat_version: crate::DAEMON_RPC_COMPAT_VERSION,
                },
            })
        }
        .instrument(span)
        .await;
        stop_log_forwarder(log_forwarder).await;
        response
    }

    async fn shutdown(
        &self,
        logs: vox::Tx<crate::machine::daemon_log::DaemonLogWireEvent>,
    ) -> Result<(), MachineError> {
        let correlation_id = next_correlation_id("shutdown");
        let log_forwarder = spawn_correlation_log_forwarder(correlation_id.clone(), logs);
        let service_name = self.config.service_name.clone();
        let span = tracing::info_span!(
            "daemon_rpc",
            correlation_id = %correlation_id,
            rpc_method = "shutdown"
        );
        let response = async move {
            tracing::info!(service_name = %service_name, "Daemon shutdown requested");
            STOP_REQUESTED.store(true, Ordering::Relaxed);
            if let Some(handle) = current_service_status_handle() {
                let _ = set_service_status(handle, SERVICE_STOP_PENDING);
            }
            Ok(())
        }
        .instrument(span)
        .await;
        stop_log_forwarder(log_forwarder).await;
        response
    }

    async fn query(
        &self,
        request: QueryPlan,
        logs: vox::Tx<crate::machine::daemon_log::DaemonLogWireEvent>,
    ) -> Result<RpcQueryResponse, MachineError> {
        let correlation_id = next_correlation_id("query");
        let log_forwarder = spawn_correlation_log_forwarder(correlation_id.clone(), logs);
        let response = self
            .run_query_in_span(request, &correlation_id)
            .await
            .map(|rows| RpcQueryResponse {
                correlation_id: correlation_id.clone(),
                rows,
            });
        stop_log_forwarder(log_forwarder).await;
        response
    }

    async fn query_stream(
        &self,
        request: QueryPlan,
        rows: vox::Tx<QueryResultRow>,
        logs: vox::Tx<crate::machine::daemon_log::DaemonLogWireEvent>,
        mut cancel: vox::Rx<u8>,
    ) -> Result<CorrelationId, MachineError> {
        let correlation_id = next_correlation_id("query");
        let log_forwarder = spawn_correlation_log_forwarder(correlation_id.clone(), logs);
        let response = self
            .run_query_stream_in_span(request, &rows, &mut cancel, &correlation_id)
            .await;
        match response {
            Ok(()) => {
                let _ = rows.close(Vec::default()).await;
                stop_log_forwarder(log_forwarder).await;
                Ok(correlation_id)
            }
            Err(error) => {
                let _ = rows.close(Vec::default()).await;
                stop_log_forwarder(log_forwarder).await;
                Err(error)
            }
        }
    }

    async fn sync(
        &self,
        request: SyncPlan,
        logs: vox::Tx<crate::machine::daemon_log::DaemonLogWireEvent>,
    ) -> Result<(), MachineError> {
        let correlation_id = next_correlation_id("sync");
        let log_forwarder = spawn_correlation_log_forwarder(correlation_id.clone(), logs);
        let drive_count = request
            .drive_letter_pattern
            .clone()
            .into_drive_letters()
            .map_err(|error| MachineError::degraded(error.to_string()))?
            .len();
        let worker = self.worker.clone();
        let span = tracing::info_span!(
            "daemon_rpc",
            correlation_id = %correlation_id,
            rpc_method = "sync"
        );
        let response = async move {
            tracing::info!(
                drive_count,
                if_exists = ?request.if_exists,
                "Starting daemon sync"
            );
            match worker.sync(request.clone(), correlation_id.clone()).await {
                Ok(()) => {
                    tracing::info!(drive_count, "Daemon sync completed");
                    Ok(())
                }
                Err(error) => {
                    tracing::warn!(error = %error.message, "Daemon sync degraded");
                    Err(error)
                }
            }
        }
        .instrument(span)
        .await;
        stop_log_forwarder(log_forwarder).await;
        response
    }

    async fn status(
        &self,
        request: StatusRequest,
        logs: vox::Tx<crate::machine::daemon_log::DaemonLogWireEvent>,
    ) -> Result<StatusResponse, MachineError> {
        let correlation_id = next_correlation_id("status");
        let log_forwarder = spawn_correlation_log_forwarder(correlation_id.clone(), logs);
        let worker = self.worker.clone();
        let config = self.config.clone();
        let span = tracing::info_span!(
            "daemon_rpc",
            correlation_id = %correlation_id,
            rpc_method = "status"
        );
        let response = async move {
            let buffered_log_count = daemon_log_hub().len();
            let snapshot = worker.snapshot();
            let published_drives = collect_published_drive_summaries_for_letters(
                &config.sync_dir,
                &request.drive_letters,
            )
            .unwrap_or_default()
            .into_iter()
            .map(|drive| crate::machine::ipc::PublishedDriveStatus {
                drive_letter: drive.drive_letter,
                mft_path: drive.mft_path.display().to_string(),
                mft_modified_at_unix_ms: drive.mft_modified_at.map(system_time_to_unix_ms),
                base_index_path: drive.base_index_path.display().to_string(),
                base_index_modified_at_unix_ms: drive
                    .base_index_modified_at
                    .map(system_time_to_unix_ms),
                overlay_index_path: drive.overlay_index_path.display().to_string(),
                overlay_index_modified_at_unix_ms: drive
                    .overlay_index_modified_at
                    .map(system_time_to_unix_ms),
                checkpoint_path: drive.checkpoint_path.display().to_string(),
                checkpoint_modified_at_unix_ms: drive
                    .checkpoint_modified_at
                    .map(system_time_to_unix_ms),
                snapshot_usn: drive
                    .checkpoint
                    .as_ref()
                    .and_then(|checkpoint| checkpoint.snapshot_usn),
                last_usn: drive
                    .checkpoint
                    .as_ref()
                    .and_then(|checkpoint| checkpoint.last_usn),
                journal_id: drive
                    .checkpoint
                    .as_ref()
                    .and_then(|checkpoint| checkpoint.journal_id),
                warning: drive.warning,
            })
            .collect();
            let status = StatusResponse {
                sync_dir: config.sync_dir.display().to_string(),
                owner_sid: config.owner_sid.clone(),
                loaded_drive_letters: snapshot.loaded_drive_letters,
                loading_drive_letters: snapshot.loading_drive_letters,
                snapshot_only_drive_letters: snapshot.snapshot_only_drive_letters,
                degraded_drives: snapshot.degraded_drives,
                active_job_count: snapshot.active_job_count,
                buffered_log_count,
                published_drives,
            };
            tracing::debug!(
                loaded_drive_count = status.loaded_drive_letters.len(),
                snapshot_only_drive_count = status.snapshot_only_drive_letters.len(),
                degraded_drive_count = status.degraded_drives.len(),
                buffered_log_count = status.buffered_log_count,
                "Collected daemon status"
            );
            Ok(status)
        }
        .instrument(span)
        .await;
        stop_log_forwarder(log_forwarder).await;
        response
    }

    async fn query_usn_journal(
        &self,
        request: crate::machine::ipc::UsnJournalRequest,
        logs: vox::Tx<crate::machine::daemon_log::DaemonLogWireEvent>,
    ) -> Result<crate::machine::ipc::UsnJournalStatus, MachineError> {
        let correlation_id = next_correlation_id("query_usn_journal");
        let log_forwarder = spawn_correlation_log_forwarder(correlation_id.clone(), logs);
        let span = tracing::info_span!(
            "daemon_rpc",
            correlation_id = %correlation_id,
            rpc_method = "query_usn_journal",
            drive = %request.drive_letter
        );
        let response = async move {
            tracing::info!(drive = %request.drive_letter, "Querying USN journal status");
            crate::machine::usn::query_journal_status(request.drive_letter)
                .map_err(|error| MachineError::degraded(error.to_string()))
        }
        .instrument(span)
        .await;
        stop_log_forwarder(log_forwarder).await;
        response
    }

    async fn stream_logs(
        &self,
        request: LogStreamRequest,
        logs: vox::Tx<crate::machine::daemon_log::DaemonLogWireEvent>,
        mut cancel: vox::Rx<u8>,
    ) -> Result<(), MachineError> {
        tracing::info!(
            replay_recent = request.replay_recent,
            follow = request.follow,
            "Attaching daemon log stream"
        );
        if request.replay_recent {
            for event in daemon_log_hub().snapshot() {
                if logs
                    .send(crate::machine::daemon_log::DaemonLogWireEvent::from(&event))
                    .await
                    .is_err()
                {
                    return Ok(());
                }
            }
        }

        if request.follow {
            let mut live_rx = daemon_log_hub().subscribe();
            loop {
                if STOP_REQUESTED.load(Ordering::Relaxed) {
                    let _ = logs
                        .send(crate::machine::daemon_log::DaemonLogWireEvent {
                            timestamp_unix_ms: crate::machine::config::current_unix_ms(),
                            level: crate::machine::daemon_log::DaemonLogLevel::Info,
                            target: module_path!().to_owned(),
                            file: Some(file!().to_owned()),
                            line: Some(line!()),
                            message:
                                "Daemon log stream closing because daemon shutdown was requested"
                                    .to_owned(),
                            request_id: 0,
                            method: "global".to_owned(),
                            correlation_id: None,
                            spans: Vec::new(),
                            fields: Vec::new(),
                        })
                        .await;
                    break;
                }
                tokio::select! {
                    cancel_result = cancel.recv() => {
                        match cancel_result {
                            Ok(Some(_) | None) => break,
                            Err(error) => {
                                tracing::warn!(error = %error, "Daemon log stream cancel channel failed");
                                break;
                            }
                        }
                    }
                    live_result = live_rx.recv() => {
                        match live_result {
                            Ok(event) => {
                                if logs
                                    .send(crate::machine::daemon_log::DaemonLogWireEvent::from(&event))
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                                tracing::warn!(
                                    skipped,
                                    "Daemon log stream subscriber lagged behind live daemon logs"
                                );
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        }
                    }
                    () = tokio::time::sleep(Duration::from_millis(250)) => {}
                }
            }
        }

        let _ = logs.close(Vec::default()).await;
        Ok(())
    }
}

fn system_time_to_unix_ms(value: std::time::SystemTime) -> u64 {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "Unix milliseconds fit in u64 for practical system lifetimes"
    )]
    {
        value
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
}

impl MachineDaemonPipeAcceptor {
    fn bind(addr: String, owner_sid: String) -> eyre::Result<Self> {
        let server = create_named_pipe_server(&addr, &owner_sid, true)?;
        Ok(Self {
            addr,
            owner_sid,
            pending: Mutex::new(server),
        })
    }

    async fn accept(&self) -> eyre::Result<DaemonPipeLink> {
        let mut guard = self.pending.lock().await;
        guard.connect().await?;
        let next = create_named_pipe_server(&self.addr, &self.owner_sid, false)?;
        let connected = std::mem::replace(&mut *guard, next);
        drop(guard);
        let (reader, writer) = tokio::io::split(connected);
        Ok(vox_stream::StreamLink::new(
            Box::new(reader),
            Box::new(writer),
        ))
    }
}

fn create_named_pipe_server(
    addr: &str,
    owner_sid: &str,
    first_pipe_instance: bool,
) -> eyre::Result<NamedPipeServer> {
    let mut security_attributes =
        crate::machine::security::named_pipe_security_attributes(owner_sid)?;
    let mut options = ServerOptions::new();
    if first_pipe_instance {
        options.first_pipe_instance(true);
    }
    // SAFETY: the security attributes are built from a live, valid security descriptor and
    // remain alive for the duration of the creation call.
    Ok(unsafe {
        options.create_with_security_attributes_raw(addr, security_attributes.as_mut_ptr())?
    })
}

fn collect_published_drive_summaries_for_letters(
    sync_dir: &std::path::Path,
    drive_letters: &[char],
) -> eyre::Result<Vec<crate::machine::status::PublishedDriveSummary>> {
    drive_letters
        .iter()
        .copied()
        .map(|drive_letter| {
            crate::machine::status::collect_published_drive_summaries(
                sync_dir,
                &crate::windows_utils::storage::DriveLetterPattern(drive_letter.to_string()),
            )
        })
        .collect::<eyre::Result<Vec<_>>>()
        .map(|summaries| summaries.into_iter().flatten().collect())
}

/// # Errors
///
/// Returns an error if the daemon runtime cannot be started.
pub fn run_daemon(service_mode: bool) -> eyre::Result<()> {
    if service_mode {
        run_windows_service_dispatcher()
    } else {
        let config = load_machine_config()?
            .wrap_err("Machine config is not installed. Run `teamy-mft install` first.")?;
        run_daemon_runtime(config)
    }
}

fn run_windows_service_dispatcher() -> eyre::Result<()> {
    let config = load_machine_config()?
        .wrap_err("Machine config is not installed. Run `teamy-mft install` first.")?;
    let mut service_name = crate::machine::security::encode_wide(&config.service_name);
    let mut table = [
        SERVICE_TABLE_ENTRYW {
            lpServiceName: windows::core::PWSTR(service_name.as_mut_ptr()),
            lpServiceProc: Some(service_main),
        },
        SERVICE_TABLE_ENTRYW::default(),
    ];

    // SAFETY: The service dispatch table is valid for this call and includes the required trailing null entry.
    unsafe { StartServiceCtrlDispatcherW(table.as_mut_ptr()) }?;
    Ok(())
}

unsafe extern "system" fn service_main(_argc: u32, _argv: *mut windows::core::PWSTR) {
    if let Err(error) = service_main_impl() {
        eprintln!("teamy-mft daemon service failed: {error:?}");
    }
}

fn service_main_impl() -> eyre::Result<()> {
    STOP_REQUESTED.store(false, Ordering::Relaxed);
    let config = load_machine_config()?
        .wrap_err("Machine config is not installed. Run `teamy-mft install` first.")?;
    let service_name = crate::machine::security::encode_wide(&config.service_name);
    // SAFETY: The service name pointer remains valid for the call and the handler function has the required ABI.
    let handle = unsafe {
        RegisterServiceCtrlHandlerExW(
            PCWSTR(service_name.as_ptr()),
            Some(service_control_handler),
            None,
        )
    }?;
    SERVICE_STATUS_HANDLE_SLOT.store(handle.0 as isize, Ordering::Relaxed);
    set_service_status(handle, SERVICE_START_PENDING)?;
    set_service_status(handle, SERVICE_RUNNING)?;
    let run_result = run_daemon_runtime(config);
    let _ = set_service_status(handle, SERVICE_STOPPED);
    SERVICE_STATUS_HANDLE_SLOT.store(0, Ordering::Relaxed);
    run_result
}

unsafe extern "system" fn service_control_handler(
    control: u32,
    _event_type: u32,
    _event_data: *mut std::ffi::c_void,
    _context: *mut std::ffi::c_void,
) -> u32 {
    match control {
        SERVICE_CONTROL_STOP | SERVICE_CONTROL_SHUTDOWN => {
            STOP_REQUESTED.store(true, Ordering::Relaxed);
            if let Some(handle) = current_service_status_handle() {
                let _ = set_service_status(handle, SERVICE_STOP_PENDING);
            }
            NO_ERROR.0
        }
        _ => NO_ERROR.0,
    }
}

fn current_service_status_handle() -> Option<SERVICE_STATUS_HANDLE> {
    let raw = SERVICE_STATUS_HANDLE_SLOT.load(Ordering::Relaxed);
    (raw != 0).then_some(SERVICE_STATUS_HANDLE(raw as *mut c_void))
}

fn set_service_status(
    handle: SERVICE_STATUS_HANDLE,
    current_state: SERVICE_STATUS_CURRENT_STATE,
) -> eyre::Result<()> {
    let controls = if current_state == SERVICE_START_PENDING {
        0
    } else {
        SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN
    };
    let status = SERVICE_STATUS {
        dwServiceType: SERVICE_WIN32_OWN_PROCESS,
        dwCurrentState: current_state,
        dwControlsAccepted: controls,
        dwWin32ExitCode: NO_ERROR.0,
        dwServiceSpecificExitCode: 0,
        dwCheckPoint: 0,
        dwWaitHint: 0,
    };
    // SAFETY: `handle` comes from the SCM and `status` is fully initialized for the duration of the call.
    unsafe { SetServiceStatus(handle, &raw const status) }?;
    Ok(())
}

fn run_daemon_runtime(config: MachineConfig) -> eyre::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let local = tokio::task::LocalSet::new();
    runtime.block_on(local.run_until(async move {
        info!(
            service_name = %config.service_name,
            sync_dir = %config.sync_dir.display(),
            pipe_name = %config.pipe_name,
            "Daemon runtime starting"
        );
        debug!("Checking machine cache protection before binding daemon pipe");
        let protection_status =
            crate::machine::security::query_path_protection_status(&config.sync_dir, &config.owner_sid)?;
        crate::machine::security::warn_if_path_protection_disabled(
            &config.sync_dir,
            &protection_status,
        );
        debug!("Machine cache protection check completed");
        let service = MachineDaemonService::new(config.clone());
        let last_activity = Arc::new(StdMutex::new(Instant::now()));
        let active_connections = Arc::new(AtomicUsize::new(0));
        let idle_timeout = Duration::from_secs(config.idle_timeout_secs);
        debug!("Binding daemon named pipe");
        let acceptor =
            MachineDaemonPipeAcceptor::bind(config.pipe_name.clone(), config.owner_sid.clone())?;
        info!(
            service_name = %config.service_name,
            pipe_name = %config.pipe_name,
            idle_timeout_secs = config.idle_timeout_secs,
            "Daemon named pipe bound and ready"
        );

        loop {
            if STOP_REQUESTED.load(Ordering::Relaxed) {
                break;
            }
            if active_connections.load(Ordering::Relaxed) == 0
                && last_daemon_activity_elapsed(&last_activity) >= idle_timeout
            {
                break;
            }

            tokio::select! {
                accept_result = acceptor.accept() => {
                    let link = accept_result?;
                    let rpc_service = service.clone();
                    mark_daemon_activity(&last_activity);
                    active_connections.fetch_add(1, Ordering::Relaxed);
                    let active_connections = Arc::clone(&active_connections);
                    let last_activity = Arc::clone(&last_activity);
                    tokio::task::spawn_local(async move {
                        let response = vox::acceptor_on(link)
                            .on_connection(crate::machine::ipc::MachineDaemonRpcDispatcher::new(rpc_service))
                            .establish::<crate::machine::ipc::MachineDaemonRpcClient>()
                            .await;
                        match response {
                            Ok(client) => {
                                tracing::debug!("Daemon RPC connection established");
                                client.caller.closed().await;
                                tracing::debug!("Daemon RPC connection closed");
                            }
                            Err(error) => {
                                tracing::warn!(error = %error, "Daemon RPC connection failed");
                            }
                        }
                        mark_daemon_activity(&last_activity);
                        active_connections.fetch_sub(1, Ordering::Relaxed);
                    });
                }
                () = tokio::time::sleep(Duration::from_millis(250)) => {}
            }
        }

        service.worker.flush();
        service.worker.stop();
        Ok(())
    }))
}

fn mark_daemon_activity(last_activity: &StdMutex<Instant>) {
    if let Ok(mut last_activity) = last_activity.lock() {
        *last_activity = Instant::now();
    }
}

fn last_daemon_activity_elapsed(last_activity: &StdMutex<Instant>) -> Duration {
    last_activity
        .lock()
        .map_or(Duration::ZERO, |last_activity| last_activity.elapsed())
}

#[cfg(test)]
mod tests {
    use super::DriveWorkerState;
    use crate::query::QueryPlan;
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::Ordering;

    #[test]
    fn cancelled_drive_query_does_not_mark_drive_snapshot_only() -> eyre::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let cancel = AtomicBool::new(true);
        let mut state = DriveWorkerState {
            drive: 'Z',
            sync_dir: temp_dir.path().to_path_buf(),
            state: None,
            snapshot_only: None,
            degraded: None,
            loading: false,
            active_jobs: 0,
        };

        let result = state.query_with_cancel(&QueryPlan::new("music"), &cancel)?;

        assert!(result.rows.is_empty());
        assert!(result.degraded.is_none());
        assert!(state.snapshot_only.is_none());
        assert!(state.degraded.is_none());
        Ok(())
    }

    #[test]
    fn drive_query_load_failure_without_cancel_reports_degraded() -> eyre::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let cancel = AtomicBool::new(false);
        let mut state = DriveWorkerState {
            drive: 'Z',
            sync_dir: temp_dir.path().to_path_buf(),
            state: None,
            snapshot_only: None,
            degraded: None,
            loading: false,
            active_jobs: 0,
        };

        let error = state
            .query_with_cancel(&QueryPlan::new("music"), &cancel)
            .expect_err("missing published files should degrade when not cancelled");

        assert!(
            error
                .message
                .contains("Drive Z has no published MFT snapshot")
        );
        assert!(!cancel.load(Ordering::Relaxed));
        Ok(())
    }
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "catch_unwind returns owned boxed panic payloads"
)]
fn machine_error_from_panic(
    context: &'static str,
    payload: Box<dyn std::any::Any + Send>,
) -> MachineError {
    let detail = if let Some(message) = payload.downcast_ref::<&'static str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        String::from("non-string panic payload")
    };
    warn!(context, detail, "Daemon request panicked");
    MachineError::degraded(format!("{context}: {detail}"))
}

/// # Errors
///
/// Returns an error if sync fails or if overlay/checkpoint sidecars cannot be written.
pub fn sync_machine_cache(
    sync_dir: &std::path::Path,
    drive_letters: &[char],
    if_exists: IfExistsOutputBehaviour,
) -> eyre::Result<MachineCacheSyncResult> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(sync_machine_cache_async(sync_dir, drive_letters, if_exists))
}

async fn sync_machine_cache_async(
    sync_dir: &std::path::Path,
    drive_letters: &[char],
    if_exists: IfExistsOutputBehaviour,
) -> eyre::Result<MachineCacheSyncResult> {
    std::fs::create_dir_all(sync_dir)?;
    let (live_drives, snapshot_cursors, skipped_drives) =
        collect_supported_drives_for_machine_sync(drive_letters);
    let drive_infos =
        resolve_drive_infos_in_dir_for_letters(sync_dir, drive_letters.iter().copied())?;
    execute_sync(drive_infos.clone(), &if_exists).await?;

    for info in drive_infos {
        let paths = published_drive_paths(sync_dir, info.drive_letter);
        if !paths.overlay_index_path.is_file() {
            crate::search_index::search_index_bytes::SearchIndexBytesMut::from_rows(
                crate::search_index::format::SearchIndexHeader::new(info.drive_letter, 0, 0),
                &[],
            )?
            .write_to_path(&paths.overlay_index_path)?;
        }
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
        save_checkpoint(&paths.checkpoint_path, &checkpoint)?;
    }

    Ok(MachineCacheSyncResult {
        synced_drives: drive_letters.to_vec(),
        live_drives,
        skipped_drives,
    })
}

fn collect_supported_drives_for_machine_sync(drive_letters: &[char]) -> SupportedDriveSyncOutcome {
    let mut supported_drives = Vec::new();
    let mut cursors = FxHashMap::default();
    let mut skipped_drives = Vec::new();
    for &drive in drive_letters {
        match VolumeUsnJournalHandle::open(drive).and_then(|journal| journal.query_cursor()) {
            Ok(cursor) => {
                supported_drives.push(drive);
                cursors.insert(drive, cursor);
            }
            Err(error) => {
                let message = error.to_string();
                warn!(
                    drive = %drive,
                    error = %message,
                    "Skipping drive for machine-managed sync because no active USN journal is available"
                );
                skipped_drives.push((drive, message));
            }
        }
    }
    (supported_drives, cursors, skipped_drives)
}
