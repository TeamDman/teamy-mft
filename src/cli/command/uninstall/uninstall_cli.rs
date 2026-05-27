use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};

#[derive(Facet, Arbitrary, PartialEq, Debug, Default)]
pub struct UninstallArgs {
    /// Delete the machine cache directory after removing the service
    #[facet(args::named, default)]
    pub purge: bool,
}

impl UninstallArgs {
    /// # Errors
    ///
    /// Returns an error if service removal fails.
    pub fn invoke(self) -> eyre::Result<()> {
        let sync_dir = crate::machine::config::load_machine_config()?
            .map(|config| config.sync_dir.into_inner());
        crate::cli::command::service::ServiceUninstallArgs { purge: self.purge }.invoke()?;
        if !self.purge
            && let Some(sync_dir) = sync_dir
        {
            println!(
                "Cache contents were preserved. Delete them manually if desired: {}",
                sync_dir.display()
            );
        }
        Ok(())
    }
}
