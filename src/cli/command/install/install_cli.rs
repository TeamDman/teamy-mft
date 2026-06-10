use arbitrary::Arbitrary;
use eyre::ensure;
use facet::Facet;
use figue::{self as args};
use tracing::info;

#[derive(Facet, Arbitrary, PartialEq, Debug, Default)]
pub struct InstallArgs {
    /// Machine-wide cache directory (defaults to `ProgramData`)
    #[facet(args::named)]
    pub sync_dir: Option<String>,

    /// Reinstall by removing any existing service registration first (requires `--daemon`)
    #[facet(args::named, default)]
    pub force: bool,

    /// Configure the machine cache without installing the daemon service
    #[facet(args::named, default)]
    pub no_daemon: bool,

    /// Install the machine daemon service in addition to configuring the machine cache
    #[facet(args::named, default)]
    pub daemon: bool,
}

impl InstallArgs {
    /// # Errors
    ///
    /// Returns an error if the daemon mode selection is invalid, machine config setup fails,
    /// or service installation fails when `--daemon` is requested.
    pub fn invoke(self) -> eyre::Result<()> {
        ensure!(
            !(self.daemon && self.no_daemon),
            "`--daemon` and `--no-daemon` cannot be used together"
        );
        ensure!(
            !self.force || self.daemon,
            "`--force` requires `--daemon`; plain `install` now configures the sync directory without installing the service"
        );

        let config = crate::cli::command::service::install_machine_config(self.sync_dir)?;
        if self.daemon {
            return crate::cli::command::service::install_machine_daemon_service(
                &config, self.force,
            );
        }

        info!("Configured machine cache at {}", config.sync_dir.display());
        println!("Configured machine cache at {}", config.sync_dir.display());
        println!("Run `teamy-mft sync` to publish initial machine-managed snapshots.");
        println!("Run `teamy-mft install --daemon` to install the machine daemon service.");
        Ok(())
    }
}
