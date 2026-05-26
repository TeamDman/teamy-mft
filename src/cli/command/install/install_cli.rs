use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};

#[derive(Facet, Arbitrary, PartialEq, Debug, Default)]
pub struct InstallArgs {
    /// Machine-wide cache directory (defaults to `ProgramData`)
    #[facet(args::named)]
    pub sync_dir: Option<String>,

    /// Reinstall by removing any existing service registration first
    #[facet(args::named, default)]
    pub force: bool,
}

impl InstallArgs {
    /// # Errors
    ///
    /// Returns an error if service installation fails.
    pub fn invoke(self) -> eyre::Result<()> {
        crate::cli::command::service::ServiceInstallArgs {
            sync_dir: self.sync_dir,
            force: self.force,
        }
        .invoke()
    }
}
