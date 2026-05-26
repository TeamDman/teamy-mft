use crate::machine::config::DEFAULT_SERVICE_NAME;
use crate::machine::ipc::load_machine_daemon_client_config;
use crate::machine::service::start_service_if_needed;
use arbitrary::Arbitrary;
use facet::Facet;

#[derive(Facet, Arbitrary, PartialEq, Debug, Default)]
pub struct ServiceStartArgs;

impl ServiceStartArgs {
    /// # Errors
    ///
    /// Returns an error if the service cannot be started.
    pub fn invoke(self) -> eyre::Result<()> {
        let config = load_machine_daemon_client_config()?;
        let service_name = if config.service_name.is_empty() {
            DEFAULT_SERVICE_NAME
        } else {
            &config.service_name
        };
        start_service_if_needed(service_name)?;
        println!("Started {service_name}");
        Ok(())
    }
}
