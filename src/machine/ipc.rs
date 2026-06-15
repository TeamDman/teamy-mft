pub use crate::daemon::CorrelationId;
pub use crate::daemon::DaemonBuildInfo;
pub use crate::daemon::DegradedDriveStatus;
pub use crate::daemon::LogStreamRequest;
pub use crate::daemon::MachineDaemonRpc;
pub use crate::daemon::MachineDaemonRpcClient;
pub use crate::daemon::MachineDaemonRpcDispatcher;
pub use crate::daemon::MachineError;
pub use crate::daemon::MachineErrorKind;
pub use crate::daemon::PingResponse;
pub use crate::daemon::PublishedDriveStatus;
pub use crate::daemon::QueryResponse as RpcQueryResponse;
pub use crate::daemon::StatusRequest;
pub use crate::daemon::StatusResponse;
pub use crate::daemon::UsnJournalRequest;
pub use crate::daemon::UsnJournalStatus;
use crate::machine::config::MachineConfig;
use crate::machine::daemon_log::DaemonLogWireEvent;
use crate::query::QueryPlan;
use crate::query::QueryResultRow;
pub use crate::sync::SyncPlan;
use std::cmp::Ordering;
use std::time::Duration;
use tracing::Instrument;

const DAEMON_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Status output needs explicit per-field compatibility booleans"
)]
pub struct DaemonCompatibility {
    pub rpc_compat_matches: bool,
    pub app_version_matches: bool,
    pub git_revision_matches: bool,
    pub build_unix_ms_matches: bool,
}

#[derive(Debug, Clone)]
pub struct ReadyDaemon {
    pub ping: PingResponse,
    pub compatibility: DaemonCompatibility,
    pub restarted: bool,
}

impl DaemonCompatibility {
    #[must_use]
    pub fn is_fully_matching(&self) -> bool {
        self.rpc_compat_matches
            && self.app_version_matches
            && self.git_revision_matches
            && self.build_unix_ms_matches
    }
}

/// # Errors
///
/// Returns an error if the daemon transport cannot be reached or the call fails outside
/// the daemon's structured machine error contract.
pub fn query(
    config: &MachineConfig,
    request: QueryPlan,
    logs: vox::Tx<DaemonLogWireEvent>,
) -> eyre::Result<Vec<QueryResultRow>> {
    with_client(config, "query", move |client| async move {
        client
            .query(request, logs)
            .await
            .map(|response| response.rows)
    })
}

/// # Errors
///
/// Returns an error if the daemon transport cannot be reached or the call fails outside
/// the daemon's structured machine error contract.
pub fn query_stream(
    config: &MachineConfig,
    request: QueryPlan,
    rows: vox::Tx<QueryResultRow>,
    logs: vox::Tx<DaemonLogWireEvent>,
    cancel: vox::Rx<u8>,
) -> eyre::Result<CorrelationId> {
    with_client(config, "query_stream", move |client| async move {
        client.query_stream(request, rows, logs, cancel).await
    })
}

/// # Errors
///
/// Returns an error if the daemon transport cannot be reached or the call fails outside
/// the daemon's structured machine error contract.
pub fn sync(
    config: &MachineConfig,
    request: SyncPlan,
    logs: vox::Tx<DaemonLogWireEvent>,
) -> eyre::Result<()> {
    with_client(config, "sync", move |client| async move {
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
    logs: vox::Tx<DaemonLogWireEvent>,
) -> eyre::Result<StatusResponse> {
    with_client(config, "status", move |client| async move {
        client.status(request, logs).await
    })
}

/// # Errors
///
/// Returns an error if the daemon transport cannot be reached or the call fails outside
/// the daemon's structured machine error contract.
pub fn query_usn_journal(
    config: &MachineConfig,
    request: UsnJournalRequest,
    logs: vox::Tx<DaemonLogWireEvent>,
) -> eyre::Result<UsnJournalStatus> {
    with_client(config, "query_usn_journal", move |client| async move {
        client.query_usn_journal(request, logs).await
    })
}

/// # Errors
///
/// Returns an error if the daemon transport cannot be reached or the call fails outside
/// the daemon's structured machine error contract.
pub fn ping(
    config: &MachineConfig,
    logs: vox::Tx<DaemonLogWireEvent>,
) -> eyre::Result<PingResponse> {
    with_client(config, "ping", move |client| async move {
        client.ping(logs).await
    })
}

/// # Errors
///
/// Returns an error if the daemon transport cannot be reached or the call fails outside
/// the daemon's structured machine error contract.
pub fn shutdown(config: &MachineConfig, logs: vox::Tx<DaemonLogWireEvent>) -> eyre::Result<()> {
    with_client(config, "shutdown", move |client| async move {
        client.shutdown(logs).await
    })
}

#[must_use]
pub fn daemon_compatibility(ping: &PingResponse) -> DaemonCompatibility {
    DaemonCompatibility {
        rpc_compat_matches: ping.build.rpc_compat_version == crate::DAEMON_RPC_COMPAT_VERSION,
        app_version_matches: ping.build.app_version == crate::APP_SEMVER,
        git_revision_matches: ping.build.git_revision == crate::APP_GIT_REVISION,
        build_unix_ms_matches: ping.build.build_unix_ms.to_string() == crate::APP_BUILD_UNIX_MS,
    }
}

/// # Errors
///
/// Returns an error if the daemon cannot be reached or reports incompatible RPC compatibility metadata.
pub fn ensure_daemon_compatible(config: &MachineConfig) -> eyre::Result<PingResponse> {
    let ping_response = ping_without_restart(config)?;
    ensure_rpc_compatibility(&ping_response)?;
    Ok(ping_response)
}

/// # Errors
///
/// Returns an error if the daemon cannot be started, restarted, or still reports an
/// incompatible RPC version after a restart attempt.
pub fn ensure_daemon_ready(config: &MachineConfig) -> eyre::Result<ReadyDaemon> {
    crate::machine::service::start_service_if_needed(&config.service_name)?;
    tracing::info!(
        service_name = %config.service_name,
        pipe_name = %config.pipe_name,
        "Sending daemon ping"
    );
    let mut ping = ping_without_restart(config)?;
    tracing::info!(
        service_name = %ping.service_name,
        daemon_app_version = %ping.build.app_version,
        daemon_git_revision = %ping.build.git_revision,
        daemon_build_unix_ms = ping.build.build_unix_ms,
        daemon_rpc_compat_version = ping.build.rpc_compat_version,
        "Received daemon pong"
    );
    let mut compatibility = daemon_compatibility(&ping);
    let mut restarted = false;

    if should_restart_for_local_build(&ping) {
        let current_exe = std::env::current_exe()?;
        if crate::machine::service::is_development_target_exe(&current_exe) {
            eyre::bail!(
                "Refusing to restart the machine daemon from a Cargo build output path: {}. \
Use the repo's .\\install.ps1 workflow to update the installed daemon binary.",
                current_exe.display()
            );
        }
        tracing::warn!(
            daemon_app_version = %ping.build.app_version,
            daemon_git_revision = %ping.build.git_revision,
            daemon_build_unix_ms = ping.build.build_unix_ms,
            cli_app_version = crate::APP_SEMVER,
            cli_git_revision = crate::APP_GIT_REVISION,
            cli_build_unix_ms = crate::APP_BUILD_UNIX_MS,
            "Restarting daemon because the running daemon build is older than the current CLI"
        );
        restart_daemon(config)?;
        tracing::info!(
            service_name = %config.service_name,
            pipe_name = %config.pipe_name,
            "Sending daemon ping after restart"
        );
        ping = ping_without_restart(config)?;
        tracing::info!(
            service_name = %ping.service_name,
            daemon_app_version = %ping.build.app_version,
            daemon_git_revision = %ping.build.git_revision,
            daemon_build_unix_ms = ping.build.build_unix_ms,
            daemon_rpc_compat_version = ping.build.rpc_compat_version,
            "Received daemon pong after restart"
        );
        compatibility = daemon_compatibility(&ping);
        restarted = true;
    }

    ensure_rpc_compatibility(&ping)?;

    Ok(ReadyDaemon {
        ping,
        compatibility,
        restarted,
    })
}

/// # Errors
///
/// Returns an error if the installed machine config exists but cannot be parsed.
pub fn load_machine_daemon_client_config() -> eyre::Result<MachineConfig> {
    crate::machine::config::load_machine_client_config()
}

/// # Errors
///
/// Returns an error if the daemon transport cannot be reached or the call fails outside
/// the daemon's structured machine error contract.
pub fn stream_logs(
    config: &MachineConfig,
    request: LogStreamRequest,
    logs: vox::Tx<DaemonLogWireEvent>,
    cancel: vox::Rx<u8>,
) -> eyre::Result<()> {
    with_client(config, "stream_logs", move |client| async move {
        client.stream_logs(request, logs, cancel).await
    })
}

fn with_client<F, Fut, T>(config: &MachineConfig, rpc_method: &'static str, f: F) -> eyre::Result<T>
where
    F: FnOnce(MachineDaemonRpcClient) -> Fut,
    Fut: std::future::Future<Output = Result<T, vox::VoxError<MachineError>>>,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let addr = format!("local://{}", config.pipe_name);
    runtime.block_on(async move {
        let connect_span = tracing::info_span!(
            "daemon_ipc_connect",
            rpc_method,
            service_name = %config.service_name,
            pipe_name = %config.pipe_name,
            timeout_ms = DAEMON_CONNECT_TIMEOUT.as_millis()
        );
        let client: MachineDaemonRpcClient = async {
            vox::connect(&addr)
                .connect_timeout(DAEMON_CONNECT_TIMEOUT)
                .wait_for_service(DAEMON_CONNECT_TIMEOUT)
                .await
        }
        .instrument(connect_span)
        .await
        .map_err(|error| eyre::eyre!("Failed connecting to daemon at {addr}: {error}"))?;
        let call_span = tracing::info_span!("daemon_ipc_call", rpc_method);
        match f(client).instrument(call_span).await {
            Ok(value) => Ok(value),
            Err(vox::VoxError::User(error)) => Err(machine_error_report(&error)),
            Err(error) => Err(eyre::eyre!("Daemon RPC call failed: {error}")),
        }
    })
}

fn ping_without_restart(config: &MachineConfig) -> eyre::Result<PingResponse> {
    with_client(config, "ping", move |client| async move {
        let (logs_tx, logs_rx) = vox::channel::<DaemonLogWireEvent>();
        let ping_response = client.ping(logs_tx).await;
        crate::machine::daemon_log::drain_stderr_logs_until_idle(
            logs_rx,
            Duration::from_millis(100),
        )
        .instrument(tracing::info_span!(
            "daemon_log_drain_until_idle",
            rpc_method = "ping"
        ))
        .await;
        ping_response
    })
}

fn restart_daemon(config: &MachineConfig) -> eyre::Result<()> {
    match shutdown_with_log_drain(config) {
        Ok(()) => {}
        Err(error) => {
            tracing::warn!(
                error = %error,
                "Daemon shutdown failed during restart; falling back to service stop"
            );
            let _ = crate::machine::service::stop_service_if_running(&config.service_name)?;
        }
    }
    crate::machine::service::wait_for_stopped(&config.service_name, Duration::from_secs(10))?;
    crate::machine::service::start_service_if_needed(&config.service_name)
}

fn shutdown_with_log_drain(config: &MachineConfig) -> eyre::Result<()> {
    with_client(config, "shutdown", move |client| async move {
        let (logs_tx, logs_rx) = vox::channel::<DaemonLogWireEvent>();
        let shutdown_response = client.shutdown(logs_tx).await;
        crate::machine::daemon_log::drain_stderr_logs_until_idle(
            logs_rx,
            Duration::from_millis(100),
        )
        .instrument(tracing::info_span!(
            "daemon_log_drain_until_idle",
            rpc_method = "shutdown"
        ))
        .await;
        shutdown_response
    })
}

fn machine_error_report(error: &MachineError) -> eyre::Report {
    eyre::eyre!("Daemon RPC failed ({:?}): {}", error.kind, error.message)
}

fn should_restart_for_local_build(ping: &PingResponse) -> bool {
    should_restart_for_build(
        crate::APP_SEMVER,
        crate::APP_GIT_REVISION,
        crate::APP_BUILD_UNIX_MS,
        ping,
    )
}

fn should_restart_for_build(
    cli_app_version: &str,
    cli_git_revision: &str,
    cli_build_unix_ms: &str,
    ping: &PingResponse,
) -> bool {
    match compare_semver(cli_app_version, &ping.build.app_version) {
        Some(Ordering::Greater) => true,
        Some(Ordering::Equal) => compare_build_unix_ms(cli_build_unix_ms, ping.build.build_unix_ms)
            .is_some_and(|ordering| {
                ordering == Ordering::Greater
                    || (ordering == Ordering::Equal && cli_git_revision != ping.build.git_revision)
            }),
        Some(Ordering::Less) | None => false,
    }
}

/// # Errors
///
/// Returns an error if the daemon RPC compatibility version does not match this CLI.
pub fn ensure_rpc_compatibility(ping: &PingResponse) -> eyre::Result<()> {
    let compatibility = daemon_compatibility(ping);
    if !compatibility.rpc_compat_matches {
        eyre::bail!(
            "Machine daemon RPC compatibility mismatch: cli rpc_compat_version={} daemon rpc_compat_version={}. Reinstall or restart the daemon with the current teamy-mft binary.",
            crate::DAEMON_RPC_COMPAT_VERSION,
            ping.build.rpc_compat_version
        );
    }
    Ok(())
}

fn compare_semver(left: &str, right: &str) -> Option<Ordering> {
    let left = semver::Version::parse(left).ok()?;
    let right = semver::Version::parse(right).ok()?;
    Some(left.cmp(&right))
}

fn compare_build_unix_ms(left: &str, right: u64) -> Option<Ordering> {
    let left = left.parse::<u64>().ok()?;
    Some(left.cmp(&right))
}

#[cfg(test)]
mod tests {
    use super::PingResponse;
    use super::ensure_rpc_compatibility;
    use super::should_restart_for_build;

    fn ping_response(
        app_version: &str,
        git_revision: &str,
        build_unix_ms: u64,
        rpc_compat_version: u32,
    ) -> PingResponse {
        PingResponse {
            service_name: "teamy-mft".to_owned(),
            build: super::DaemonBuildInfo {
                app_version: app_version.to_owned(),
                git_revision: git_revision.to_owned(),
                build_unix_ms,
                rpc_compat_version,
            },
        }
    }

    fn mismatched_rpc_compat_version() -> u32 {
        if crate::DAEMON_RPC_COMPAT_VERSION == 0 {
            1
        } else {
            0
        }
    }

    #[test]
    fn restart_is_required_when_cli_version_is_newer() {
        let ping = ping_response("1.2.2", "git-a", 1, crate::DAEMON_RPC_COMPAT_VERSION);

        assert!(should_restart_for_build("1.2.3", "git-a", "2", &ping));
    }

    #[test]
    fn restart_is_required_when_versions_match_but_git_revisions_differ() {
        let ping = ping_response("1.2.3", "git-daemon", 2, crate::DAEMON_RPC_COMPAT_VERSION);

        assert!(should_restart_for_build("1.2.3", "git-cli", "2", &ping));
        assert!(!should_restart_for_build("1.2.3", "git-cli", "1", &ping));
    }

    #[test]
    fn restart_is_required_when_versions_match_but_cli_build_is_newer() {
        let ping = ping_response("1.2.3", "git-a", 2, crate::DAEMON_RPC_COMPAT_VERSION);

        assert!(should_restart_for_build("1.2.3", "git-a", "3", &ping));
    }

    #[test]
    fn restart_is_not_required_when_daemon_version_is_newer() {
        let ping = ping_response("1.2.4", "git-a", 2, crate::DAEMON_RPC_COMPAT_VERSION);

        assert!(!should_restart_for_build("1.2.3", "git-a", "3", &ping));
    }

    #[test]
    fn invalid_semver_values_do_not_trigger_restart() {
        let valid_ping = ping_response("1.2.3", "git-a", 2, crate::DAEMON_RPC_COMPAT_VERSION);
        let invalid_ping =
            ping_response("not-a-semver", "git-a", 2, crate::DAEMON_RPC_COMPAT_VERSION);

        assert!(!should_restart_for_build(
            "not-a-semver",
            "git-a",
            "3",
            &valid_ping
        ));
        assert!(!should_restart_for_build(
            "1.2.3",
            "git-a",
            "3",
            &invalid_ping
        ));
    }

    #[test]
    fn equal_versions_still_error_when_rpc_compatibility_mismatches() {
        let ping = ping_response("1.2.3", "git-a", 2, mismatched_rpc_compat_version());

        assert!(!should_restart_for_build("1.2.3", "git-a", "2", &ping));

        let error = ensure_rpc_compatibility(&ping)
            .expect_err("rpc compatibility mismatch should still fail");

        assert!(
            error
                .to_string()
                .contains("Machine daemon RPC compatibility mismatch")
        );
    }
}
