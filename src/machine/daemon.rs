use crate::cli::command::sync::IfExistsOutputBehaviour;
use crate::cli::command::sync::SyncCommand;
use crate::cli::command::sync::index::SyncIndexArgs;
use crate::cli::command::sync::resolve_drive_infos_in_dir_for_letters;
use crate::machine::config::MachineConfig;
use crate::machine::config::PublishedCheckpoint;
use crate::machine::config::current_unix_ms;
use crate::machine::config::load_checkpoint;
use crate::machine::config::load_machine_config;
use crate::machine::config::published_drive_paths;
use crate::machine::config::save_checkpoint;
use crate::machine::daemon_log::daemon_log_hub;
use crate::machine::daemon_log::spawn_correlation_log_forwarder;
use crate::machine::daemon_log::stop_log_forwarder;
use crate::machine::ipc::CorrelationId;
use crate::machine::ipc::DegradedDriveStatus;
use crate::machine::ipc::IfExistsDto;
use crate::machine::ipc::LogStreamRequest;
use crate::machine::ipc::MachineDaemonRpc;
use crate::machine::ipc::MachineError;
use crate::machine::ipc::PingResponse;
use crate::machine::ipc::QueryRequest;
use crate::machine::ipc::QueryStreamResponse;
use crate::machine::ipc::RpcQueryResponse;
use crate::machine::ipc::StatusRequest;
use crate::machine::ipc::StatusResponse;
use crate::machine::ipc::SyncModeDto;
use crate::machine::ipc::SyncRequest;
use crate::machine::live_drive_state::LiveDriveState;
use crate::machine::usn::JournalCursor;
use crate::machine::usn::VolumeUsnJournal;
use crate::query::QueryIgnoreRules;
use crate::query::QueryPlan;
use crate::query::matching_row_indices_for_rule;
use crate::search_index::format::SEARCH_INDEX_VERSION;
use crate::search_index::search_index_bytes::SearchIndexBytes;
use futures::FutureExt;
use rustc_hash::FxHashMap;
use std::collections::BTreeMap;
use std::ffi::c_void;
use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicIsize;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::net::windows::named_pipe::NamedPipeServer;
use tokio::net::windows::named_pipe::ServerOptions;
use tokio::sync::Mutex;
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
    Option<FxHashMap<char, JournalCursor>>,
    Vec<(char, String)>,
);

#[derive(Debug)]
struct DaemonRuntimeState {
    owner_sid: String,
    cache_root: std::path::PathBuf,
    drives: FxHashMap<char, LiveDriveState>,
    degraded: FxHashMap<char, String>,
}

#[derive(Debug, Clone)]
struct MachineDaemonService {
    config: MachineConfig,
    state: Arc<Mutex<DaemonRuntimeState>>,
}

impl DaemonRuntimeState {
    fn new(config: &MachineConfig) -> Self {
        Self {
            owner_sid: config.owner_sid.clone(),
            cache_root: config.cache_root.clone().into_inner(),
            drives: FxHashMap::default(),
            degraded: FxHashMap::default(),
        }
    }

    fn query(
        &mut self,
        request: &QueryRequest,
    ) -> Result<Vec<crate::query::IndexedPathRow>, MachineError> {
        let mut rows = Vec::new();
        let mut queried_drives = 0usize;
        let mut degraded_drives = Vec::new();
        for &drive in &request.drive_letters {
            if let Err(error) = self.refresh_drive(drive) {
                match self.query_published_drive(drive, request) {
                    Ok(drive_rows) => {
                        queried_drives += 1;
                        rows.extend(drive_rows);
                        degraded_drives.push((
                            drive,
                            format!(
                                "{}; served published cache for this drive instead",
                                error.message
                            ),
                        ));
                    }
                    Err(fallback_error) => degraded_drives.push((
                        drive,
                        format!(
                            "{}; published cache fallback also failed: {fallback_error}",
                            error.message
                        ),
                    )),
                }
                continue;
            }
            let mut per_drive_request = request.clone();
            per_drive_request.drive_letters = vec![drive];
            per_drive_request.limit = 0;
            match self.drive_mut(drive)?.query(&per_drive_request) {
                Ok(drive_rows) => {
                    queried_drives += 1;
                    rows.extend(drive_rows);
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
                "Daemon query skipped degraded drives"
            );
        }

        if request.limit > 0 && rows.len() > request.limit {
            rows.truncate(request.limit);
        }
        Ok(rows)
    }

    fn query_published_drive(
        &self,
        drive: char,
        request: &QueryRequest,
    ) -> eyre::Result<Vec<crate::query::IndexedPathRow>> {
        let paths = published_drive_paths(&self.cache_root, drive);
        let query_plan = QueryPlan::parse_inputs(&request.query)?;
        let query_scope = resolve_query_scope(request.query_scope.as_deref())?;
        let ignore_rules = match QueryIgnoreRules::discover_for_drive_letters(
            &[drive],
            &self.cache_root,
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
        let mut rows = query_published_index_path(&paths.base_index_path, &query_plan)?;

        if paths.overlay_index_path.is_file() {
            rows = merge_index_rows(
                rows,
                query_published_index_path(&paths.overlay_index_path, &query_plan)?,
            );
        }

        Ok(rows
            .into_iter()
            .filter_map(|mut row| {
                if !should_include_indexed_row(
                    request.include_deleted,
                    request.only_deleted,
                    row.has_deleted_entries,
                ) {
                    return None;
                }
                if !should_include_scope(&row.path, query_scope.as_ref()) {
                    return None;
                }
                row.is_ignored = ignore_rules
                    .as_ref()
                    .is_some_and(|rules| rules.is_ignored_path(Path::new(&row.path)));
                should_include_ignored_row(
                    request.show_ignored,
                    request.only_ignored,
                    row.is_ignored,
                )
                .then_some(row)
            })
            .collect())
    }

    fn status_response(&self, buffered_log_count: usize, drive_letters: &[char]) -> StatusResponse {
        let published_drives =
            collect_published_drive_summaries_for_letters(&self.cache_root, drive_letters)
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
        StatusResponse {
            cache_root: self.cache_root.display().to_string(),
            owner_sid: self.owner_sid.clone(),
            loaded_drive_letters: self.drives.keys().copied().collect(),
            degraded_drives: self
                .degraded
                .iter()
                .map(|(&drive_letter, message)| DegradedDriveStatus {
                    drive_letter,
                    message: message.clone(),
                })
                .collect(),
            buffered_log_count,
            published_drives,
        }
    }

    async fn sync(&mut self, request: SyncRequest) -> Result<(), MachineError> {
        self.flush_dirty_drives();
        info!(
            drives = ?request.drive_letters,
            mode = ?request.mode,
            if_exists = ?request.if_exists,
            "daemon sync request starting"
        );
        crate::machine::security::restrict_path_to_owner(&self.cache_root, &self.owner_sid)
            .map_err(|error| MachineError::degraded(error.to_string()))?;
        repair_published_drive_permissions(
            &self.cache_root,
            &self.owner_sid,
            &request.drive_letters,
        )
        .map_err(|error| MachineError::degraded(error.to_string()))?;
        let sync_result = sync_machine_cache_async(
            &self.cache_root,
            &request.drive_letters,
            request.mode,
            request.if_exists,
        )
        .await
        .map_err(|error| MachineError::degraded(error.to_string()))?;

        debug!(
            synced_drives = ?sync_result.synced_drives,
            live_drives = ?sync_result.live_drives,
            skipped_drives = ?sync_result.skipped_drives,
            "Machine-managed sync completed"
        );

        for &drive in &request.drive_letters {
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
            let state = self.load_drive_state(drive).map_err(|error| {
                MachineError::degraded(format!(
                    "Drive {drive} could not be loaded for live query: {error}"
                ))
            })?;
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
        let paths = published_drive_paths(&self.cache_root, drive);
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
        LiveDriveState::load(&self.cache_root, paths)
    }
}

fn format_degraded_query_drives(degraded_drives: &[(char, String)]) -> String {
    degraded_drives
        .iter()
        .map(|(drive, message)| format!("{drive}: {message}"))
        .collect::<Vec<_>>()
        .join("; ")
}

fn query_published_index_path(
    path: &Path,
    query_plan: &QueryPlan,
) -> eyre::Result<Vec<crate::query::IndexedPathRow>> {
    let bytes = std::fs::read(path)?;
    let parsed_index = SearchIndexBytes::new(&bytes).parse_trusted_for_query()?;
    let matched_row_indices = query_plan
        .matching_row_indices(&|rule| matching_row_indices_for_rule(&parsed_index, rule))?;
    let mut rows = Vec::with_capacity(matched_row_indices.len());
    for row_index in matched_row_indices {
        let row = parsed_index.row_view(row_index as usize)?;
        rows.push(crate::query::IndexedPathRow {
            path: row.path(),
            has_deleted_entries: row.has_deleted_entries,
            is_ignored: false,
        });
    }
    Ok(rows)
}

fn merge_index_rows(
    base_rows: Vec<crate::query::IndexedPathRow>,
    overlay_rows: Vec<crate::query::IndexedPathRow>,
) -> Vec<crate::query::IndexedPathRow> {
    let mut merged = BTreeMap::<String, crate::query::IndexedPathRow>::new();
    for row in base_rows {
        merged.insert(row.path.clone(), row);
    }
    for row in overlay_rows {
        merged.insert(row.path.clone(), row);
    }
    merged.into_values().collect()
}

#[derive(Debug, Clone)]
struct QueryScope {
    root: PathBuf,
    include_descendants: bool,
}

fn resolve_query_scope(scope: Option<&str>) -> eyre::Result<Option<QueryScope>> {
    let Some(scope) = scope else {
        return Ok(None);
    };

    let root = dunce::canonicalize(scope)?;
    Ok(Some(QueryScope {
        include_descendants: root.is_dir(),
        root,
    }))
}

fn lowercase_path_components(path: &Path) -> Vec<String> {
    let path = path.as_os_str().to_string_lossy().replace('/', "\\");
    let path = path
        .strip_prefix(r"\\?\UNC\")
        .map_or_else(|| path.clone(), |rest| format!(r"\\{rest}"));
    let path = path
        .strip_prefix(r"\\?\")
        .map_or_else(|| path.clone(), ToString::to_string);

    path.split('\\')
        .filter(|component| !component.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

fn path_matches_scope(path: &Path, scope: &QueryScope) -> bool {
    if cfg!(windows) {
        let path_components = lowercase_path_components(path);
        let scope_components = lowercase_path_components(&scope.root);
        return if scope.include_descendants {
            path_components.starts_with(&scope_components)
        } else {
            path_components == scope_components
        };
    }

    if scope.include_descendants {
        path.starts_with(&scope.root)
    } else {
        path == scope.root
    }
}

fn should_include_scope(path: &str, scope: Option<&QueryScope>) -> bool {
    let Some(scope) = scope else {
        return true;
    };
    path_matches_scope(Path::new(path), scope)
}

fn should_include_indexed_row(
    include_deleted: bool,
    only_deleted: bool,
    has_deleted_entries: bool,
) -> bool {
    if only_deleted {
        return has_deleted_entries;
    }
    include_deleted || !has_deleted_entries
}

fn should_include_ignored_row(show_ignored: bool, only_ignored: bool, is_ignored: bool) -> bool {
    if only_ignored {
        return is_ignored;
    }
    show_ignored || !is_ignored
}

impl MachineDaemonService {
    fn new(config: MachineConfig) -> Self {
        let state = Arc::new(Mutex::new(DaemonRuntimeState::new(&config)));
        Self { config, state }
    }

    async fn run_query_in_span(
        &self,
        request: QueryRequest,
        correlation_id: &CorrelationId,
    ) -> Result<Vec<crate::query::IndexedPathRow>, MachineError> {
        let state = Arc::clone(&self.state);
        let request_for_body = request.clone();
        let span = tracing::info_span!(
            "daemon_rpc",
            correlation_id = %correlation_id,
            rpc_method = "query"
        );
        async move {
            tracing::info!(
                query_groups = request_for_body.query.len(),
                drive_count = request_for_body.drive_letters.len(),
                limit = request_for_body.limit,
                "Running daemon query"
            );
            let mut state = state.lock().await;
            match std::panic::catch_unwind(AssertUnwindSafe(|| state.query(&request_for_body))) {
                Ok(Ok(rows)) => {
                    tracing::info!(matched_rows = rows.len(), "Daemon query completed");
                    Ok(rows)
                }
                Ok(Err(error)) => {
                    tracing::warn!(error = %error.message, "Daemon query degraded");
                    Err(error)
                }
                Err(payload) => {
                    let error = machine_error_from_panic("query request panicked", payload);
                    tracing::error!(error = %error.message, "Daemon query panicked");
                    Err(error)
                }
            }
        }
        .instrument(span)
        .await
    }
}

fn next_correlation_id(method: &str) -> CorrelationId {
    let _ = method;
    let _ = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    CorrelationId::new()
}

fn repair_published_drive_permissions(
    cache_root: &std::path::Path,
    owner_sid: &str,
    drive_letters: &[char],
) -> eyre::Result<()> {
    for &drive in drive_letters {
        let paths = published_drive_paths(cache_root, drive);
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
        request: QueryRequest,
        logs: vox::Tx<crate::machine::daemon_log::DaemonLogWireEvent>,
    ) -> Result<RpcQueryResponse, MachineError> {
        let correlation_id = next_correlation_id("query");
        let log_forwarder = spawn_correlation_log_forwarder(correlation_id.clone(), logs);
        let response = self
            .run_query_in_span(request, &correlation_id)
            .await
            .map(|rows| crate::machine::ipc::convert_indexed_rows(rows, correlation_id.clone()));
        stop_log_forwarder(log_forwarder).await;
        response
    }

    async fn query_stream(
        &self,
        request: QueryRequest,
        rows: vox::Tx<teamy_mft_daemon_rpc::IndexedPathRowDto>,
        logs: vox::Tx<crate::machine::daemon_log::DaemonLogWireEvent>,
    ) -> Result<QueryStreamResponse, MachineError> {
        let correlation_id = next_correlation_id("query");
        let log_forwarder = spawn_correlation_log_forwarder(correlation_id.clone(), logs);
        let response = self.run_query_in_span(request, &correlation_id).await;
        match response {
            Ok(matched_rows) => {
                for row in matched_rows {
                    if rows
                        .send(teamy_mft_daemon_rpc::IndexedPathRowDto {
                            path: row.path,
                            has_deleted_entries: row.has_deleted_entries,
                            is_ignored: row.is_ignored,
                        })
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                let _ = rows.close(Vec::default()).await;
                stop_log_forwarder(log_forwarder).await;
                Ok(QueryStreamResponse { correlation_id })
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
        request: SyncRequest,
        logs: vox::Tx<crate::machine::daemon_log::DaemonLogWireEvent>,
    ) -> Result<(), MachineError> {
        let correlation_id = next_correlation_id("sync");
        let log_forwarder = spawn_correlation_log_forwarder(correlation_id.clone(), logs);
        let drive_count = request.drive_letters.len();
        let state = Arc::clone(&self.state);
        let span = tracing::info_span!(
            "daemon_rpc",
            correlation_id = %correlation_id,
            rpc_method = "sync"
        );
        let response = async move {
            tracing::info!(
                drive_count,
                mode = ?request.mode,
                if_exists = ?request.if_exists,
                "Starting daemon sync"
            );
            match AssertUnwindSafe(async {
                let mut state = state.lock().await;
                state.sync(request.clone()).await
            })
            .catch_unwind()
            .await
            {
                Ok(Ok(())) => {
                    tracing::info!(drive_count, "Daemon sync completed");
                    Ok(())
                }
                Ok(Err(error)) => {
                    tracing::warn!(error = %error.message, "Daemon sync degraded");
                    Err(error)
                }
                Err(payload) => {
                    let error = machine_error_from_panic("sync request panicked", payload);
                    tracing::error!(error = %error.message, "Daemon sync panicked");
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
        let state = Arc::clone(&self.state);
        let span = tracing::info_span!(
            "daemon_rpc",
            correlation_id = %correlation_id,
            rpc_method = "status"
        );
        let response = async move {
            let buffered_log_count = daemon_log_hub().len();
            let status = state
                .lock()
                .await
                .status_response(buffered_log_count, &request.drive_letters);
            tracing::debug!(
                loaded_drive_count = status.loaded_drive_letters.len(),
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
    cache_root: &std::path::Path,
    drive_letters: &[char],
) -> eyre::Result<Vec<crate::machine::status::PublishedDriveSummary>> {
    drive_letters
        .iter()
        .copied()
        .map(|drive_letter| {
            crate::machine::status::collect_published_drive_summaries(
                cache_root,
                &teamy_windows::storage::DriveLetterPattern(drive_letter.to_string()),
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
        let config = load_machine_config()?.ok_or_else(|| {
            eyre::eyre!("Machine config is not installed. Run `teamy-mft install` first.")
        })?;
        run_daemon_runtime(config)
    }
}

fn run_windows_service_dispatcher() -> eyre::Result<()> {
    let config = load_machine_config()?.ok_or_else(|| {
        eyre::eyre!("Machine config is not installed. Run `teamy-mft install` first.")
    })?;
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
    let config = load_machine_config()?.ok_or_else(|| {
        eyre::eyre!("Machine config is not installed. Run `teamy-mft install` first.")
    })?;
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
    runtime.block_on(async move {
        crate::machine::security::restrict_path_to_owner(&config.cache_root, &config.owner_sid)?;
        let service = MachineDaemonService::new(config.clone());
        let mut last_activity = std::time::Instant::now();
        let idle_timeout = Duration::from_secs(config.idle_timeout_secs);
        let acceptor =
            MachineDaemonPipeAcceptor::bind(config.pipe_name.clone(), config.owner_sid.clone())?;

        loop {
            if STOP_REQUESTED.load(Ordering::Relaxed) {
                break;
            }
            if last_activity.elapsed() >= idle_timeout {
                break;
            }

            tokio::select! {
                accept_result = acceptor.accept() => {
                    let link = accept_result?;
                    let rpc_service = service.clone();
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
                    last_activity = std::time::Instant::now();
                }
                () = tokio::time::sleep(Duration::from_millis(250)) => {
                    service.state.lock().await.refresh_loaded_drives();
                }
            }
        }

        service.state.lock().await.flush_dirty_drives();
        Ok(())
    })
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
    cache_root: &std::path::Path,
    drive_letters: &[char],
    mode: SyncModeDto,
    if_exists: IfExistsDto,
) -> eyre::Result<MachineCacheSyncResult> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(sync_machine_cache_async(
        cache_root,
        drive_letters,
        mode,
        if_exists,
    ))
}

async fn sync_machine_cache_async(
    cache_root: &std::path::Path,
    drive_letters: &[char],
    mode: SyncModeDto,
    if_exists: IfExistsDto,
) -> eyre::Result<MachineCacheSyncResult> {
    std::fs::create_dir_all(cache_root)?;
    let effective_mode = if matches!(mode, SyncModeDto::Mft) {
        info!(
            drives = ?drive_letters,
            "Machine-managed MFT sync upgrades to full sync so published query state stays coherent"
        );
        SyncModeDto::Both
    } else {
        mode
    };
    let (live_drives, snapshot_cursors, skipped_drives) =
        collect_supported_drives_for_machine_sync(drive_letters, effective_mode);
    let drive_infos =
        resolve_drive_infos_in_dir_for_letters(cache_root, drive_letters.iter().copied())?;
    let if_exists = match if_exists {
        IfExistsDto::Skip => IfExistsOutputBehaviour::Skip,
        IfExistsDto::Overwrite => IfExistsOutputBehaviour::Overwrite,
        IfExistsDto::Abort => IfExistsOutputBehaviour::Abort,
    };
    let sync_command = match effective_mode {
        SyncModeDto::Index => SyncCommand::Index(SyncIndexArgs),
        SyncModeDto::Both => SyncCommand::Both,
        SyncModeDto::Mft => unreachable!("effective mode normalizes Mft to Both"),
    };
    sync_command.invoke(drive_infos.clone(), &if_exists).await?;

    for info in drive_infos {
        let paths = published_drive_paths(cache_root, info.drive_letter);
        if !paths.overlay_index_path.is_file() {
            crate::search_index::search_index_bytes::SearchIndexBytesMut::from_rows(
                crate::search_index::format::SearchIndexHeader::new(info.drive_letter, 0, 0),
                &[],
            )?
            .write_to_path(&paths.overlay_index_path)?;
        }
        match effective_mode {
            SyncModeDto::Index => {
                if load_checkpoint(&paths.checkpoint_path)?.is_none() {
                    let checkpoint = PublishedCheckpoint {
                        published_at_unix_ms: current_unix_ms(),
                        ..PublishedCheckpoint::empty(info.drive_letter, SEARCH_INDEX_VERSION)
                    };
                    save_checkpoint(&paths.checkpoint_path, &checkpoint)?;
                }
            }
            SyncModeDto::Both => {
                let cursor = snapshot_cursors
                    .as_ref()
                    .and_then(|cursors| cursors.get(&info.drive_letter))
                    .copied();
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
            SyncModeDto::Mft => unreachable!("machine sync Mft mode is normalized to Both"),
        }
    }

    Ok(MachineCacheSyncResult {
        synced_drives: drive_letters.to_vec(),
        live_drives,
        skipped_drives,
    })
}

fn collect_supported_drives_for_machine_sync(
    drive_letters: &[char],
    mode: SyncModeDto,
) -> SupportedDriveSyncOutcome {
    if !matches!(mode, SyncModeDto::Both) {
        return (drive_letters.to_vec(), None, Vec::new());
    }

    let mut supported_drives = Vec::new();
    let mut cursors = FxHashMap::default();
    let mut skipped_drives = Vec::new();
    for &drive in drive_letters {
        match VolumeUsnJournal::open(drive).and_then(|journal| journal.query_cursor()) {
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
    (supported_drives, Some(cursors), skipped_drives)
}
