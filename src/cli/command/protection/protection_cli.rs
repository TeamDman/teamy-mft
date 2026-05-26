use crate::cli::command::protection::disable::ProtectionDisableArgs;
use crate::cli::command::protection::enable::ProtectionEnableArgs;
use crate::cli::command::protection::status::ProtectionStatusArgs;
use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};

#[derive(Facet, Arbitrary, PartialEq, Debug)]
pub struct ProtectionArgs {
    #[facet(args::subcommand)]
    pub command: ProtectionCommand,
}

#[derive(Facet, Arbitrary, PartialEq, Debug)]
#[repr(u8)]
#[facet(rename_all = "kebab-case")]
pub enum ProtectionCommand {
    /// Restore daemon-owned cache ACLs for sensitive MFT artifacts
    Enable(ProtectionEnableArgs),
    /// Temporarily allow broad read access to machine cache artifacts for local development
    Disable(ProtectionDisableArgs),
    /// Show machine cache protection state
    Status(ProtectionStatusArgs),
}

impl Default for ProtectionCommand {
    fn default() -> Self {
        Self::Status(ProtectionStatusArgs)
    }
}

impl ProtectionArgs {
    /// # Errors
    ///
    /// Returns an error if the selected protection subcommand fails.
    pub fn invoke(self) -> eyre::Result<()> {
        match self.command {
            ProtectionCommand::Enable(args) => args.invoke(),
            ProtectionCommand::Disable(args) => args.invoke(),
            ProtectionCommand::Status(args) => args.invoke(),
        }
    }
}
