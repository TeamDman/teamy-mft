use crate::machine::config::MachineConfig;
use crate::machine::daemon_log::DaemonLogEvent;
use crate::query::IndexedPathRow;
use std::time::Duration;
pub use teamy_mft_daemon_rpc::DaemonBuildInfo;
pub use teamy_mft_daemon_rpc::DegradedDriveStatus;
pub use teamy_mft_daemon_rpc::IfExistsDto;
pub use teamy_mft_daemon_rpc::LogStreamRequest;
pub use teamy_mft_daemon_rpc::MachineDaemonRpc;
pub use teamy_mft_daemon_rpc::MachineDaemonRpcClient;
pub use teamy_mft_daemon_rpc::MachineDaemonRpcDispatcher;
pub use teamy_mft_daemon_rpc::MachineError;
pub use teamy_mft_daemon_rpc::MachineErrorKind;
pub use teamy_mft_daemon_rpc::PingResponse;
pub use teamy_mft_daemon_rpc::QueryRequest;
pub use teamy_mft_daemon_rpc::QueryResponse as RpcQueryResponse;
pub use teamy_mft_daemon_rpc::StatusRequest;
pub use teamy_mft_daemon_rpc::StatusResponse;
pub use teamy_mft_daemon_rpc::SyncModeDto;
pub use teamy_mft_daemon_rpc::SyncRequest;

const DAEMON_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonCompatibility {
    pub rpc_compat_matches: bool,
    pub app_version_matches: bool,
    pub git_revision_matches: bool,
}

impl DaemonCompatibility {
    #[must_use]
    pub fn is_fully_matching(&self) -> bool {
        self.rpc_compat_matches && self.app_version_matches && self.git_revision_matches
    }
}

/// # Errors
///
/// Returns an error if the daemon transport cannot be reached or the call fails outside
/// the daemon's structured machine error contract.
pub fn query(
    config: &MachineConfig,
    request: QueryRequest,
    logs: vox::Tx<DaemonLogEvent>,
) -> eyre::Result<Result<Vec<IndexedPathRow>, MachineError>> {
    with_client(config, move |client| async move {
        client
            .query(request, logs)
            .await
            .map(convert_query_response)
    })
}

/// # Errors
///
/// Returns an error if the daemon transport cannot be reached or the call fails outside
/// the daemon's structured machine error contract.
pub fn query_stream(
    config: &MachineConfig,
    request: QueryRequest,
    rows: vox::Tx<teamy_mft_daemon_rpc::IndexedPathRowDto>,
    logs: vox::Tx<DaemonLogEvent>,
) -> eyre::Result<Result<(), MachineError>> {
    with_client(config, move |client| async move {
        client.query_stream(request, rows, logs).await
    })
}

/// # Errors
///
/// Returns an error if the daemon transport cannot be reached or the call fails outside
/// the daemon's structured machine error contract.
pub fn sync(
    config: &MachineConfig,
    request: SyncRequest,
    logs: vox::Tx<DaemonLogEvent>,
) -> eyre::Result<Result<(), MachineError>> {
    with_client(config, move |client| async move {
        client.sync(request, logs).await
    })
}

/// # Errors
///
/// Returns an error if the daemon transport cannot be reached or the call fails outside
/// the daemon's structured machine error contract.
pub fn status(
    config: &MachineConfig,
    request: StatusRequest,
    logs: vox::Tx<DaemonLogEvent>,
) -> eyre::Result<Result<StatusResponse, MachineError>> {
    with_client(config, move |client| async move {
        client.status(request, logs).await
    })
}

/// # Errors
///
/// Returns an error if the daemon transport cannot be reached or the call fails outside
/// the daemon's structured machine error contract.
pub fn ping(
    config: &MachineConfig,
    logs: vox::Tx<DaemonLogEvent>,
) -> eyre::Result<Result<PingResponse, MachineError>> {
    with_client(config, move |client| async move { client.ping(logs).await })
}

#[must_use]
pub fn daemon_compatibility(ping: &PingResponse) -> DaemonCompatibility {
    DaemonCompatibility {
        rpc_compat_matches: ping.build.rpc_compat_version == crate::DAEMON_RPC_COMPAT_VERSION,
        app_version_matches: ping.build.app_version == crate::APP_SEMVER,
        git_revision_matches: ping.build.git_revision == crate::APP_GIT_REVISION,
    }
}

/// # Errors
///
/// Returns an error if the daemon cannot be reached or reports incompatible RPC compatibility metadata.
pub fn ensure_daemon_compatible(config: &MachineConfig) -> eyre::Result<PingResponse> {
    let (logs_tx, logs_rx) = vox::channel::<DaemonLogEvent>();
    let log_drain = crate::machine::daemon_log::spawn_stderr_log_drain(logs_rx);
    let ping_response = ping(config, logs_tx)?;
    let _ = log_drain.join();
    let ping_response = ping_response.map_err(|error| eyre::eyre!(error.message))?;
    let compatibility = daemon_compatibility(&ping_response);
    if !compatibility.rpc_compat_matches {
        eyre::bail!(
            "Machine daemon RPC compatibility mismatch: cli rpc_compat_version={} daemon rpc_compat_version={}. Reinstall or restart the daemon with the current teamy-mft binary.",
            crate::DAEMON_RPC_COMPAT_VERSION,
            ping_response.build.rpc_compat_version
        );
    }
    Ok(ping_response)
}

/// # Errors
///
/// Returns an error if the daemon transport cannot be reached or the call fails outside
/// the daemon's structured machine error contract.
pub fn stream_logs(
    config: &MachineConfig,
    request: LogStreamRequest,
    logs: vox::Tx<DaemonLogEvent>,
    cancel: vox::Rx<u8>,
) -> eyre::Result<Result<(), MachineError>> {
    with_client(config, move |client| async move {
        client.stream_logs(request, logs, cancel).await
    })
}

fn with_client<F, Fut, T>(config: &MachineConfig, f: F) -> eyre::Result<Result<T, MachineError>>
where
    F: FnOnce(MachineDaemonRpcClient) -> Fut,
    Fut: std::future::Future<Output = Result<T, vox::VoxError<MachineError>>>,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let addr = format!("local://{}", config.pipe_name);
    runtime.block_on(async move {
        let client: MachineDaemonRpcClient = vox::connect(&addr)
            .connect_timeout(DAEMON_CONNECT_TIMEOUT)
            .wait_for_service(DAEMON_CONNECT_TIMEOUT)
            .await
            .map_err(|error| eyre::eyre!("Failed connecting to daemon at {addr}: {error}"))?;
        match f(client).await {
            Ok(value) => Ok(Ok(value)),
            Err(vox::VoxError::User(error)) => Ok(Err(error)),
            Err(error) => Err(eyre::eyre!("Daemon RPC call failed: {error}")),
        }
    })
}

fn convert_query_response(response: teamy_mft_daemon_rpc::QueryResponse) -> Vec<IndexedPathRow> {
    response
        .rows
        .into_iter()
        .map(|row| IndexedPathRow {
            path: row.path,
            has_deleted_entries: row.has_deleted_entries,
            is_ignored: row.is_ignored,
        })
        .collect()
}

#[must_use]
pub fn convert_indexed_rows(rows: Vec<IndexedPathRow>) -> teamy_mft_daemon_rpc::QueryResponse {
    teamy_mft_daemon_rpc::QueryResponse {
        rows: rows
            .into_iter()
            .map(|row| teamy_mft_daemon_rpc::IndexedPathRowDto {
                path: row.path,
                has_deleted_entries: row.has_deleted_entries,
                is_ignored: row.is_ignored,
            })
            .collect(),
    }
}
