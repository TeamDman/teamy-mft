pub mod command;
pub mod global_args;
use crate::cli::command::Command;
use crate::cli::global_args::GlobalArgs;
use arbitrary::Arbitrary;
use facet::Facet;
use figue::FigueBuiltins;
use figue::{self as args};

/// Teamy MFT command-line interface.
#[derive(Facet, Arbitrary, Debug, Default)]
// tool[impl cli.help.describes-environment]
pub struct Cli {
    #[facet(flatten)]
    pub global_args: GlobalArgs,

    #[facet(flatten)]
    #[arbitrary(default)]
    pub builtins: FigueBuiltins,

    #[facet(args::subcommand)]
    pub command: Command,
}

impl PartialEq for Cli {
    fn eq(&self, other: &Self) -> bool {
        self.global_args == other.global_args && self.command == other.command
    }
}

impl Cli {
    /// Invoke the CLI with the parsed arguments.
    ///
    /// # Errors
    ///
    /// Returns an error if the command execution fails.
    pub fn invoke(self) -> eyre::Result<()> {
        self.command.invoke()
    }
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use crate::cli::command::Command;

    #[test]
    fn status_accepts_drive_long_alias() {
        let cli: Cli = figue::from_slice(&["status", "--drive", "CD"]).unwrap();

        let Command::Status(args) = cli.command else {
            panic!("expected status command");
        };
        assert_eq!(args.drive_letter_pattern.as_ref(), "CD");
    }

    #[test]
    fn rules_accepts_drive_long_alias() {
        let args: crate::cli::command::rules::RulesArgs =
            figue::from_slice(&["list", "--drive", "CD"]).unwrap();

        let crate::cli::command::rules::RulesCommand::List(args) = args.command else {
            panic!("expected rules list command");
        };
        assert_eq!(args.drive_letter_pattern.as_ref(), "CD");
    }

    #[test]
    fn sync_accepts_drive_long_alias() {
        let cli: Cli = figue::from_slice(&["sync", "--drive", "CD"]).unwrap();

        let Command::Sync(args) = cli.command else {
            panic!("expected sync command");
        };
        assert_eq!(args.plan.drive_letter_pattern.as_ref(), "CD");
    }

    #[test]
    fn query_accepts_drive_long_alias() {
        let cli: Cli = figue::from_slice(&["query", "flowers", "--drive", "CD"]).unwrap();

        let Command::Query(args) = cli.command else {
            panic!("expected query command");
        };
        assert_eq!(args.plan.drive_letter_pattern.as_ref(), "CD");
    }

    #[test]
    fn profile_accepts_profiles_alias() {
        let canonical: Cli = figue::from_slice(&["profile", "list"]).unwrap();
        let alias: Cli = figue::from_slice(&["profiles", "list"]).unwrap();

        assert_eq!(alias, canonical);
    }
}
