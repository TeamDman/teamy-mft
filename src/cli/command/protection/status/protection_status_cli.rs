use arbitrary::Arbitrary;
use facet::Facet;

#[derive(Facet, Arbitrary, PartialEq, Debug, Default)]
pub struct ProtectionStatusArgs;

impl ProtectionStatusArgs {
    /// # Errors
    ///
    /// Returns an error if the machine config or cache ACL cannot be read.
    pub fn invoke(self) -> eyre::Result<()> {
        let config = crate::machine::config::load_required_machine_config()?;
        let status = crate::machine::security::query_path_protection_status(
            &config.cache_root,
            &config.owner_sid,
        )?;

        crate::machine::security::warn_if_path_protection_disabled(&config.cache_root, &status);

        println!("machine-cache-root={}", config.cache_root.display());
        crate::machine::security::print_path_protection_status(&status);
        Ok(())
    }
}
