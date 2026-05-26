use crate::machine::config::DEFAULT_SERVICE_NAME;
use crate::machine::config::load_machine_config;
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
        let service_state = query_service_state(&service_name)?;
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
        if let Some(config) = config {
            println!("machine-cache-root={}", config.cache_root.display());
            println!("machine-pipe-name={}", config.pipe_name);
        }
        Ok(())
    }
}
