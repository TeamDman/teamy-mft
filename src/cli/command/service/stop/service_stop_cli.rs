use crate::machine::config::DEFAULT_SERVICE_NAME;
use crate::machine::ipc::load_machine_daemon_client_config;
use crate::machine::service::stop_service_if_running;
use arbitrary::Arbitrary;
use facet::Facet;

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
        stop_service_if_running(service_name)?;
        println!("Stopped {service_name}");
        Ok(())
    }
}
