use crate::cli::command::service::install::ServiceInstallArgs;
use crate::cli::command::service::run::ServiceRunArgs;
use crate::cli::command::service::start::ServiceStartArgs;
use crate::cli::command::service::status::ServiceStatusArgs;
use crate::cli::command::service::stop::ServiceStopArgs;
use crate::cli::command::service::uninstall::ServiceUninstallArgs;
use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};

#[derive(Facet, Arbitrary, PartialEq, Debug)]
pub struct ServiceArgs {
    #[facet(args::subcommand)]
    pub command: ServiceCommand,
}

#[derive(Facet, Arbitrary, PartialEq, Debug)]
#[repr(u8)]
#[facet(rename_all = "kebab-case")]
pub enum ServiceCommand {
    /// Install the machine-wide Windows service and shared cache
    Install(ServiceInstallArgs),
    /// Uninstall the machine-wide Windows service and optionally purge its cache
    Uninstall(ServiceUninstallArgs),
    /// Start the machine-wide Windows service
    Start(ServiceStartArgs),
    /// Stop the machine-wide Windows service if it is running
    Stop(ServiceStopArgs),
    /// Show service registration and runtime status
    Status(ServiceStatusArgs),
    /// Internal daemon runtime entrypoint
    Run(ServiceRunArgs),
}

impl Default for ServiceCommand {
    fn default() -> Self {
        Self::Status(ServiceStatusArgs)
    }
}

impl ServiceArgs {
    /// # Errors
    ///
    /// Returns an error if the selected service subcommand fails.
    pub fn invoke(self) -> eyre::Result<()> {
        match self.command {
            ServiceCommand::Install(args) => args.invoke(),
            ServiceCommand::Uninstall(args) => args.invoke(),
            ServiceCommand::Start(args) => args.invoke(),
            ServiceCommand::Stop(args) => args.invoke(),
            ServiceCommand::Status(args) => args.invoke(),
            ServiceCommand::Run(args) => args.invoke(),
        }
    }
}
