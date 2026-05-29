use crate::cli::command::fsutil::usn::create_journal::FsutilUsnCreateJournalArgs;
use crate::cli::command::fsutil::usn::query_journal::FsutilUsnQueryJournalArgs;
use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};

#[derive(Facet, Arbitrary, PartialEq, Debug)]
pub struct FsutilUsnArgs {
    #[facet(args::subcommand)]
    pub command: FsutilUsnCommand,
}

#[derive(Facet, Arbitrary, PartialEq, Debug)]
#[repr(u8)]
#[facet(rename_all = "kebab-case")]
pub enum FsutilUsnCommand {
    /// Query an NTFS volume's USN journal
    QueryJournal(FsutilUsnQueryJournalArgs),
    /// Create or resize an NTFS volume's USN journal
    CreateJournal(FsutilUsnCreateJournalArgs),
}

impl FsutilUsnArgs {
    /// # Errors
    ///
    /// Returns an error if the selected USN subcommand fails.
    pub fn invoke(self) -> eyre::Result<()> {
        match self.command {
            FsutilUsnCommand::QueryJournal(args) => args.invoke(),
            FsutilUsnCommand::CreateJournal(args) => args.invoke(),
        }
    }
}
