use crate::cli::command::profile::list::ProfileListArgs;
use crate::cli::command::profile::reset::ProfileResetArgs;
use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};

#[derive(Facet, Arbitrary, PartialEq, Debug)]
pub struct ProfileArgs {
    #[facet(args::subcommand)]
    pub command: ProfileCommand,
}

#[derive(Facet, Arbitrary, PartialEq, Debug)]
#[repr(u8)]
#[facet(rename_all = "kebab-case")]
pub enum ProfileCommand {
    /// List discovered query rule profiles
    List(ProfileListArgs),
    /// Disable discovered query rule files for one profile by renaming them
    Reset(ProfileResetArgs),
}

impl ProfileArgs {
    /// # Errors
    ///
    /// Returns an error if the selected profile subcommand fails.
    pub fn invoke(self) -> eyre::Result<()> {
        match self.command {
            ProfileCommand::List(args) => args.invoke(),
            ProfileCommand::Reset(args) => args.invoke(),
        }
    }
}
