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
        let cache_root = crate::machine::config::load_machine_config()?
            .map(|config| config.cache_root.into_inner());
        crate::cli::command::service::ServiceUninstallArgs { purge: self.purge }.invoke()?;
        if !self.purge
            && let Some(cache_root) = cache_root
        {
            println!(
                "Cache contents were preserved. Delete them manually if desired: {}",
                cache_root.display()
            );
        }
        Ok(())
    }
}
