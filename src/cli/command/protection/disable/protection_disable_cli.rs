use arbitrary::Arbitrary;
use facet::Facet;
use crate::windows_utils::elevation::ensure_elevated;

#[derive(Facet, Arbitrary, PartialEq, Debug, Default)]
pub struct ProtectionDisableArgs;

impl ProtectionDisableArgs {
    /// # Errors
    ///
    /// Returns an error if elevation is unavailable or cache ACL repair fails.
    pub fn invoke(self) -> eyre::Result<()> {
        ensure_elevated()?;
        let config = crate::machine::config::load_required_machine_config()?;
        crate::machine::security::allow_development_reads(&config.cache_root)?;
        println!("machine-protection-enabled=false");
        println!("machine-cache-root={}", config.cache_root.display());
        println!(
            "machine-protection-warning=development reads are enabled for sensitive MFT cache artifacts"
        );
        Ok(())
    }
}
