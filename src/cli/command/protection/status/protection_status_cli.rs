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
            &config.sync_dir,
            &config.owner_sid,
        )?;

        crate::machine::security::warn_if_path_protection_disabled(&config.sync_dir, &status);

        println!("machine-cache-root={}", config.sync_dir.display());
        crate::machine::security::print_path_protection_status(&status);
        Ok(())
    }
}
