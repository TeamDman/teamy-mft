use arbitrary::Arbitrary;
use facet::Facet;
use crate::windows_utils::elevation::ensure_elevated;

#[derive(Facet, Arbitrary, PartialEq, Debug, Default)]
pub struct ProtectionEnableArgs;

impl ProtectionEnableArgs {
    /// # Errors
    ///
    /// Returns an error if elevation is unavailable or cache ACL repair fails.
    pub fn invoke(self) -> eyre::Result<()> {
        ensure_elevated()?;
        let config = crate::machine::config::load_required_machine_config()?;
        crate::machine::security::restrict_path_to_owner(&config.cache_root, &config.owner_sid)?;
        println!("machine-protection-enabled=true");
        println!("machine-cache-root={}", config.cache_root.display());
        Ok(())
    }
}
