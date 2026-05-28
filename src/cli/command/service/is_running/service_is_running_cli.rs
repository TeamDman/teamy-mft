use crate::machine::config::load_machine_config;
use arbitrary::Arbitrary;
use facet::Facet;
use tracing::debug;
use tracing::info;

#[derive(Facet, Arbitrary, PartialEq, Debug, Default)]
pub struct ServiceIsRunningArgs;

impl ServiceIsRunningArgs {
    /// # Errors
    ///
    /// This command exits with status 1 when the daemon is not running or cannot be queried.
    pub fn invoke(self) -> eyre::Result<()> {
        let is_running = load_machine_config().ok().flatten().is_some_and(|config| {
            let (logs_tx, logs_rx) =
                vox::channel::<crate::machine::daemon_log::DaemonLogWireEvent>();
            let log_drain = crate::machine::daemon_log::spawn_stderr_log_drain(logs_rx);
            let result = crate::machine::ipc::ping(&config, logs_tx);
            drop(log_drain);
            if let Err(error) = &result {
                debug!(error = %error, "daemon ping failed while checking running state");
            }
            result.is_ok()
        });
        info!(is_running, "daemon status");
        if is_running {
            println!("Daemon is running.");
        } else {
            std::process::exit(1);
        }
        Ok(())
    }
}
