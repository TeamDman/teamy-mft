use crate::cli::command::rules::add::RulesAddArgs;
use crate::cli::command::rules::list::RulesListArgs;
use crate::cli::command::rules::remove::RulesRemoveArgs;
use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};

#[derive(Facet, Arbitrary, PartialEq, Eq, Debug, Default)]
#[repr(u8)]
#[facet(rename_all = "kebab-case")]
pub enum RulesMutationDirective {
    #[default]
    Include,
    Exclude,
    DefaultInclude,
    DefaultExclude,
}

#[derive(Facet, Arbitrary, PartialEq, Debug)]
pub struct RuleArgs {
    #[facet(args::subcommand)]
    pub command: RulesCommand,
}

#[derive(Facet, Arbitrary, PartialEq, Debug)]
#[repr(u8)]
#[facet(rename_all = "kebab-case")]
pub enum RulesCommand {
    /// Ensure one rule exists in a discovered rules file, a new cwd rules file, or a selected `--rules-file`
    Add(RulesAddArgs),
    /// List effective `.teamy_mft_rules` files and directives for one profile
    List(RulesListArgs),
    /// Ensure one rule is absent from discovered rules files or a selected `--rules-file`
    Remove(RulesRemoveArgs),
}

impl RuleArgs {
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
