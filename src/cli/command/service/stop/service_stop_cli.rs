use crate::machine::config::DEFAULT_SERVICE_NAME;
use crate::machine::ipc::load_machine_daemon_client_config;
use crate::machine::ipc::shutdown as shutdown_daemon;
use crate::machine::service::WindowsServiceState;
use crate::machine::service::query_service_state;
use crate::machine::service::stop_service_if_running;
use arbitrary::Arbitrary;
use facet::Facet;
use tracing::info;
use tracing::warn;

#[derive(Facet, Arbitrary, PartialEq, Debug, Default)]
pub struct ServiceStopArgs;

impl ServiceStopArgs {
    /// # Errors
    ///
    /// Returns an error if the service cannot be stopped.
    pub fn invoke(self) -> eyre::Result<()> {
        let config = load_machine_daemon_client_config()?;
        let service_name = if config.service_name.is_empty() {
            DEFAULT_SERVICE_NAME
        } else {
            &config.service_name
        };
        let was_running = match query_service_state(service_name)? {
            WindowsServiceState::Running | WindowsServiceState::StartPending => {
                let (logs_tx, logs_rx) =
                    vox::channel::<crate::machine::daemon_log::DaemonLogEvent>();
                let log_drain = crate::machine::daemon_log::spawn_stderr_log_drain(logs_rx);
                match shutdown_daemon(&config, logs_tx) {
                    Ok(Ok(())) => {
                        let _ = log_drain.join();
                        wait_for_service_to_stop(service_name)?;
                        true
                    }
                    Ok(Err(error)) => {
                        let _ = log_drain.join();
                        warn!(
                            service_name,
                            error = %error.message,
                            "Daemon shutdown RPC failed; falling back to service stop"
                        );
                        stop_service_if_running(service_name)?
                    }
                    Err(error) => {
                        let _ = log_drain.join();
                        warn!(
                            service_name,
                            error = %error,
                            "Daemon shutdown transport failed; falling back to service stop"
                        );
                        stop_service_if_running(service_name)?
                    }
                }
            }
            WindowsServiceState::Stopped
            | WindowsServiceState::Missing
            | WindowsServiceState::Unknown(_) => false,
        };
        info!(
            service_name,
            was_running, "Machine daemon is now no longer running"
        );
        println!("Stopped {service_name}");
        Ok(())
    }
}

fn wait_for_service_to_stop(service_name: &str) -> eyre::Result<()> {
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_secs(10) {
        match query_service_state(service_name)? {
            WindowsServiceState::Stopped | WindowsServiceState::Missing => return Ok(()),
            WindowsServiceState::Running
            | WindowsServiceState::StartPending
            | WindowsServiceState::Unknown(_) => {
                std::thread::sleep(std::time::Duration::from_millis(150));
            }
        }
    }

    eyre::bail!("Timed out waiting for {service_name} to stop after daemon shutdown request")
}
