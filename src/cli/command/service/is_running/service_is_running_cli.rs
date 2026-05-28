use crate::machine::config::DEFAULT_SERVICE_NAME;
use crate::machine::config::load_machine_config;
use crate::machine::service::WindowsServiceState;
use crate::machine::service::query_service_state;
use arbitrary::Arbitrary;
use facet::Facet;
use tracing::info;

#[derive(Facet, Arbitrary, PartialEq, Debug, Default)]
pub struct ServiceIsRunningArgs;

impl ServiceIsRunningArgs {
    /// # Errors
    ///
    /// This command exits with status 1 when the daemon is not running or cannot be queried.
    pub fn invoke(self) -> eyre::Result<()> {
        let is_running = load_machine_config()
            .ok()
            .flatten()
            .map_or_else(
                || query_service_state(DEFAULT_SERVICE_NAME),
                |config| query_service_state(&config.service_name),
            )
            .is_ok_and(|state| matches!(state, WindowsServiceState::Running));
        info!(is_running, "daemon status");
        if is_running {
            println!("Daemon is running.");
        } else {
            std::process::exit(1);
        }
        Ok(())
    }
}
