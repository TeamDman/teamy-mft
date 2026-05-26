use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};

#[derive(Facet, Arbitrary, PartialEq, Debug, Default)]
pub struct ServiceRunArgs {
    /// Run as the Windows Service entrypoint
    #[facet(args::named, default)]
    pub service: bool,
}

impl ServiceRunArgs {
    /// # Errors
    ///
    /// Returns an error if the daemon runtime cannot be started.
    pub fn invoke(self) -> eyre::Result<()> {
        crate::machine::daemon::run_daemon(self.service)
    }
}
