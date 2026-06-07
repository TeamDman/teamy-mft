use crate::cli::command::rules::add::RulesAddArgs;
use crate::cli::command::rules::list::RulesListArgs;
use crate::cli::command::rules::remove::RulesRemoveArgs;
use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};

#[derive(Facet, Arbitrary, PartialEq, Debug)]
pub struct RulesArgs {
    #[facet(args::subcommand)]
    pub command: RulesCommand,
}

#[derive(Facet, Arbitrary, PartialEq, Debug)]
#[repr(u8)]
#[facet(rename_all = "kebab-case")]
pub enum RulesCommand {
    /// Append one rule to the managed rules file for a profile
    Add(RulesAddArgs),
    /// List effective `.teamy_mft_rules` files and directives for one profile
    List(RulesListArgs),
    /// Remove one managed rule line from a profile by line number
    Remove(RulesRemoveArgs),
}

impl RulesArgs {
    /// # Errors
    ///
    /// Returns an error if the selected rules subcommand fails.
    pub fn invoke(self) -> eyre::Result<()> {
        match self.command {
            RulesCommand::Add(args) => args.invoke(),
            RulesCommand::List(args) => args.invoke(),
            RulesCommand::Remove(args) => args.invoke(),
        }
    }
}
