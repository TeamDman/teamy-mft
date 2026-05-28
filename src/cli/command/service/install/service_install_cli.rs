use crate::machine::config::DEFAULT_SERVICE_NAME;
use crate::machine::config::MachineConfig;
use crate::machine::config::machine_config_path;
use crate::machine::config::machine_root_dir;
use crate::machine::config::save_machine_config;
use crate::machine::security::allow_machine_config_reads;
use crate::machine::security::current_user_sid_string;
use crate::machine::security::restrict_path_to_owner;
use crate::machine::service::WindowsServiceState;
use crate::machine::service::install_windows_service;
use crate::machine::service::query_service_state;
use crate::machine::service::uninstall_windows_service;
use crate::windows_utils::elevation::ensure_elevated;
use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};
use tracing::info;

#[derive(Facet, Arbitrary, PartialEq, Debug, Default)]
pub struct ServiceInstallArgs {
    /// Machine-wide cache directory (defaults to `ProgramData`)
    #[facet(args::named)]
    pub sync_dir: Option<String>,

    /// Reinstall by removing any existing service registration first
    #[facet(args::named, default)]
    pub force: bool,
}

impl ServiceInstallArgs {
    /// # Errors
    ///
    /// Returns an error if elevation, service registration, or machine config setup fails.
    pub fn invoke(self) -> eyre::Result<()> {
        let requested_sync_dir = self.sync_dir.clone();
        match query_service_state(DEFAULT_SERVICE_NAME)? {
            WindowsServiceState::Missing => {}
            _ if self.force => {
                ensure_elevated()?;
                let current_exe = std::env::current_exe()?;
                reject_development_target_exe(&current_exe)?;
                let owner_sid = current_user_sid_string()?;
                let sync_dir = requested_sync_dir
                    .clone()
                    .map(resolve_sync_dir)
                    .transpose()?;
                let config = MachineConfig::new(owner_sid.clone(), sync_dir);
                uninstall_windows_service(&config.service_name)?;
            }
            _ => {
                eyre::bail!(
                    "Service {} is already installed. Re-run with --force or run `teamy-mft service uninstall` first.",
                    DEFAULT_SERVICE_NAME
                );
            }
        }

        ensure_elevated()?;
        let current_exe = std::env::current_exe()?;
        reject_development_target_exe(&current_exe)?;
        let owner_sid = current_user_sid_string()?;
        let sync_dir = requested_sync_dir.map(resolve_sync_dir).transpose()?;
        let config = MachineConfig::new(owner_sid.clone(), sync_dir);
        let machine_root = machine_root_dir();
        std::fs::create_dir_all(&machine_root)?;
        std::fs::create_dir_all(&config.sync_dir)?;
        restrict_path_to_owner(&machine_root, &owner_sid)?;
        restrict_path_to_owner(&config.sync_dir, &owner_sid)?;
        save_machine_config(&config)?;
        allow_machine_config_reads(&machine_root, &machine_config_path())?;
        install_windows_service(&current_exe, &config)?;
        info!("Installed machine daemon at {}", config.sync_dir.display());
        println!(
            "Installed machine daemon cache at {}",
            config.sync_dir.display()
        );
        println!("Run `teamy-mft sync` to publish initial machine-managed snapshots.");
        Ok(())
    }
}

fn resolve_sync_dir(path: String) -> eyre::Result<std::path::PathBuf> {
    let path = std::path::PathBuf::from(path);
    if path.is_absolute() {
        return Ok(path);
    }
    Ok(std::env::current_dir()?.join(path))
}

fn reject_development_target_exe(path: &std::path::Path) -> eyre::Result<()> {
    let components = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_ascii_lowercase())
        .collect::<Vec<_>>();
    let is_cargo_target_build = components
        .windows(2)
        .any(|pair| pair[0] == "target" && (pair[1] == "debug" || pair[1] == "release"));
    if is_cargo_target_build {
        eyre::bail!(
            "Refusing to install the machine service from a Cargo build output path: {}. \
Build and invoke the real binary instead, for example via the repo's install.ps1 workflow.",
            path.display()
        );
    }
    Ok(())
}
