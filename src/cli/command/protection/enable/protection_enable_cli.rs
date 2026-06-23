use crate::cli::command::protection::ProtectionTarget;
use crate::windows_utils::elevation::ensure_elevated;
use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};

#[derive(Facet, Arbitrary, PartialEq, Debug, Default)]
pub struct ProtectionEnableArgs {
    /// Cache artifact set to protect
    #[facet(args::positional, default)]
    pub target: Option<ProtectionTarget>,
}

impl ProtectionEnableArgs {
    /// # Errors
    ///
    /// Returns an error if elevation is unavailable or cache ACL repair fails.
    pub fn invoke(self) -> eyre::Result<()> {
        ensure_elevated()?;
        let config = crate::machine::config::load_required_machine_config()?;
        let target = self.target.unwrap_or(ProtectionTarget::All);
        match target {
            ProtectionTarget::All => {
                crate::machine::security::restrict_path_to_owner(
                    &config.sync_dir,
                    &config.owner_sid,
                )?;
            }
            ProtectionTarget::Mft | ProtectionTarget::Index => {
                for path in target.existing_paths(&config.sync_dir)? {
                    crate::machine::security::restrict_path_to_owner(&path, &config.owner_sid)?;
                    println!("machine-protection-path={}", path.display());
                }
            }
        }
        println!("machine-protection-enabled=true");
        println!("machine-protection-target={target}");
        println!("machine-cache-root={}", config.sync_dir.display());
        Ok(())
    }
}
