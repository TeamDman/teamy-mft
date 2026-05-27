use crate::machine::config::DEFAULT_SERVICE_NAME;
use crate::machine::config::load_machine_config;
use crate::machine::ipc::load_machine_daemon_client_config;
use crate::machine::service::query_service_state;
use arbitrary::Arbitrary;
use facet::Facet;

#[derive(Facet, Arbitrary, PartialEq, Debug, Default)]
pub struct ServiceStatusArgs;

impl ServiceStatusArgs {
    /// # Errors
    ///
    /// Returns an error if the service state cannot be queried.
    pub fn invoke(self) -> eyre::Result<()> {
        let config = load_machine_config()?;
        let service_name = config.as_ref().map_or_else(
            || String::from(DEFAULT_SERVICE_NAME),
            |config| config.service_name.clone(),
        );
        let mut service_state = query_service_state(&service_name)?;
        let daemon_ready = load_machine_daemon_client_config()
            .and_then(|config| crate::machine::ipc::ensure_daemon_ready(&config))
            .ok();
        if daemon_ready.is_some() {
            service_state = crate::machine::service::WindowsServiceState::Running;
        }
        println!("machine-service-name={service_name}");
        println!(
            "machine-service-state={}",
            match service_state {
                crate::machine::service::WindowsServiceState::Missing => "missing",
                crate::machine::service::WindowsServiceState::Stopped => "stopped",
                crate::machine::service::WindowsServiceState::StartPending => "start-pending",
                crate::machine::service::WindowsServiceState::Running => "running",
                crate::machine::service::WindowsServiceState::Unknown(_) => "unknown",
            }
        );
        println!("machine-daemon-reachable={}", daemon_ready.is_some());
        if let Some(config) = config {
            println!("machine-cache-root={}", config.sync_dir.display());
            println!("machine-pipe-name={}", config.pipe_name);
        }
        if let Some(ready_daemon) = daemon_ready {
            println!(
                "machine-daemon-app-version={}",
                ready_daemon.ping.build.app_version
            );
            println!(
                "machine-daemon-git-revision={}",
                ready_daemon.ping.build.git_revision
            );
            println!(
                "machine-daemon-build-unix-ms={}",
                ready_daemon.ping.build.build_unix_ms
            );
            println!(
                "machine-daemon-rpc-compat-version={}",
                ready_daemon.ping.build.rpc_compat_version
            );
            println!("machine-daemon-cli-app-version={}", crate::APP_SEMVER);
            println!(
                "machine-daemon-cli-git-revision={}",
                crate::APP_GIT_REVISION
            );
            println!(
                "machine-daemon-cli-build-unix-ms={}",
                crate::APP_BUILD_UNIX_MS
            );
            println!(
                "machine-daemon-cli-rpc-compat-version={}",
                crate::DAEMON_RPC_COMPAT_VERSION
            );
            println!(
                "machine-daemon-build-fully-matching={}",
                ready_daemon.compatibility.is_fully_matching()
            );
        }
        Ok(())
    }
}
