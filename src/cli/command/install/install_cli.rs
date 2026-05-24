use crate::machine::config::{MachineConfig, machine_root_dir, save_machine_config};
use crate::machine::daemon::sync_machine_cache;
use crate::machine::ipc::{IfExistsDto, SyncModeDto};
use crate::machine::security::{current_user_sid_string, restrict_path_to_owner};
use crate::machine::service::install_windows_service;
use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};
use teamy_windows::elevation::ensure_elevated;
use tracing::info;

#[derive(Facet, Arbitrary, PartialEq, Debug, Default)]
pub struct InstallArgs {
    /// Machine-wide cache directory (defaults to ProgramData)
    #[facet(args::named)]
    pub sync_dir: Option<String>,
}

impl InstallArgs {
    /// # Errors
    ///
    /// Returns an error if elevation, service registration, or the initial sync fails.
    pub fn invoke(self) -> eyre::Result<()> {
        ensure_elevated()?;
        let owner_sid = current_user_sid_string()?;
        let cache_root = self.sync_dir.map(resolve_cache_root).transpose()?;
        let config = MachineConfig::new(owner_sid.clone(), cache_root);
        std::fs::create_dir_all(machine_root_dir())?;
        std::fs::create_dir_all(&config.cache_root)?;
        save_machine_config(&config)?;
        restrict_path_to_owner(&machine_root_dir(), &owner_sid)?;
        restrict_path_to_owner(&config.cache_root, &owner_sid)?;
        install_windows_service(&std::env::current_exe()?, &config)?;
        let drive_letters =
            teamy_windows::storage::DriveLetterPattern::default().into_drive_letters()?;
        sync_machine_cache(
            &config.cache_root,
            &drive_letters,
            SyncModeDto::Both,
            IfExistsDto::Overwrite,
        )?;
        info!(
            "Installed machine daemon at {}",
            config.cache_root.display()
        );
        println!(
            "Installed machine daemon cache at {}",
            config.cache_root.display()
        );
        Ok(())
    }
}

fn resolve_cache_root(path: String) -> eyre::Result<std::path::PathBuf> {
    let path = std::path::PathBuf::from(path);
    if path.is_absolute() {
        return Ok(path);
    }
    Ok(std::env::current_dir()?.join(path))
}
