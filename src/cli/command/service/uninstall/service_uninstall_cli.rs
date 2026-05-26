use crate::machine::config::DEFAULT_SERVICE_NAME;
use crate::machine::config::load_machine_config;
use crate::machine::config::machine_config_path;
use crate::machine::config::machine_root_dir;
use crate::machine::service::uninstall_windows_service;
use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};
use teamy_windows::elevation::ensure_elevated;

#[derive(Facet, Arbitrary, PartialEq, Debug, Default)]
pub struct ServiceUninstallArgs {
    /// Delete the machine cache directory after removing the service
    #[facet(args::named, default)]
    pub purge: bool,
}

impl ServiceUninstallArgs {
    /// # Errors
    ///
    /// Returns an error if elevation or service removal fails.
    pub fn invoke(self) -> eyre::Result<()> {
        ensure_elevated()?;
        let config = load_machine_config()?;
        let service_name = config.as_ref().map_or_else(
            || String::from(DEFAULT_SERVICE_NAME),
            |config| config.service_name.clone(),
        );
        uninstall_windows_service(&service_name)?;

        let config_path = machine_config_path();
        if config_path.is_file() {
            std::fs::remove_file(&config_path)?;
        }
        if self.purge && machine_root_dir().exists() {
            std::fs::remove_dir_all(machine_root_dir())?;
        }

        println!("Uninstalled {service_name}");
        Ok(())
    }
}
