use crate::cli::command::protection::ProtectionTarget;
use crate::windows_utils::elevation::ensure_elevated;
use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};

#[derive(Facet, Arbitrary, PartialEq, Debug, Default)]
pub struct ProtectionDisableArgs {
    /// Cache artifact set to make readable
    #[facet(args::positional, default)]
    pub target: Option<ProtectionTarget>,
}

impl ProtectionDisableArgs {
    /// # Errors
    ///
    /// Returns an error if elevation is unavailable or cache ACL repair fails.
    pub fn invoke(self) -> eyre::Result<()> {
        ensure_elevated()?;
        let config = crate::machine::config::load_required_machine_config()?;
        let target = self.target.unwrap_or(ProtectionTarget::Index);
        match target {
            ProtectionTarget::All => {
                crate::machine::security::allow_development_reads(&config.sync_dir)?;
            }
            ProtectionTarget::Mft | ProtectionTarget::Index => {
                for path in target.existing_paths(&config.sync_dir)? {
                    crate::machine::security::allow_development_reads(&path)?;
                    println!("machine-protection-path={}", path.display());
                }
            }
        }
        println!("machine-protection-enabled=false");
        println!("machine-protection-target={target}");
        println!("machine-cache-root={}", config.sync_dir.display());
        match target {
            ProtectionTarget::All | ProtectionTarget::Mft => {
                println!(
                    "machine-protection-warning=development reads are enabled for sensitive MFT cache artifacts"
                );
            }
            ProtectionTarget::Index => {
                println!(
                    "machine-protection-warning=development reads are enabled for query index artifacts only"
                );
            }
        }
        Ok(())
    }
}
