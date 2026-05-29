use crate::cli::command::fsutil::usn::FsutilUsnArgs;
use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};

#[derive(Facet, Arbitrary, PartialEq, Debug)]
pub struct FsutilArgs {
    #[facet(args::subcommand)]
    pub command: FsutilCommand,
}

#[derive(Facet, Arbitrary, PartialEq, Debug)]
#[repr(u8)]
#[facet(rename_all = "kebab-case")]
pub enum FsutilCommand {
    /// Manage NTFS USN change journals using native Windows APIs
    Usn(FsutilUsnArgs),
}

impl FsutilArgs {
    /// # Errors
    ///
    /// Returns an error if the selected fsutil-compatible subcommand fails.
    pub fn invoke(self) -> eyre::Result<()> {
        match self.command {
            FsutilCommand::Usn(args) => args.invoke(),
        }
    }
}
