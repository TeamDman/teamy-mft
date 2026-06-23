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
        let args: crate::cli::command::rules::RuleArgs =
            figue::from_slice(&["list", "--drive", "CD"]).unwrap();

        let crate::cli::command::rules::RulesCommand::List(args) = args.command else {
            panic!("expected rules list command");
        };
        assert_eq!(args.drive_letter_pattern.as_ref(), "CD");
    }

    #[test]
    fn rules_add_accepts_rules_file_and_drive_alias() {
        let args: crate::cli::command::rules::RuleArgs = figue::from_slice(&[
            "add",
            "--profile",
            "my-profile-123",
            "--rules-file",
            r".\teamy-mft-rules.my-profile-123.teamy_mft_rules",
            "--drive",
            "CD",
            "include",
            r"C:\Repos\teamy-mft\src\**",
        ])
        .unwrap();

        let crate::cli::command::rules::RulesCommand::Add(args) = args.command else {
            panic!("expected rules add command");
        };
        assert_eq!(args.drive_letter_pattern.as_ref(), "CD");
        assert_eq!(
            args.rules_file.as_deref(),
            Some(r".\teamy-mft-rules.my-profile-123.teamy_mft_rules")
        );
    }

    #[test]
    fn rules_remove_parses_directive_based_shape() {
        let args: crate::cli::command::rules::RuleArgs = figue::from_slice(&[
            "remove",
            "--profile",
            "my-profile-123",
            "--order",
            "10",
            "exclude",
            r"C:\Repos\secret\**",
        ])
        .unwrap();

        let crate::cli::command::rules::RulesCommand::Remove(args) = args.command else {
            panic!("expected rules remove command");
        };
        assert_eq!(args.profile.as_deref(), Some("my-profile-123"));
        assert_eq!(args.order, Some(10));
    }

    #[test]
    fn move_accepts_source_and_directory_target() {
        let cli: Cli = figue::from_slice(&["move", r".\a.teamy_mft_rules", ".\\rules\\"]).unwrap();

        let Command::Move(args) = cli.command else {
            panic!("expected move command");
        };
        assert_eq!(args.source, r".\a.teamy_mft_rules");
        assert_eq!(args.destination, ".\\rules\\");
    }

    #[test]
    fn mv_alias_matches_move() {
        let canonical: Cli =
            figue::from_slice(&["move", r".\a.teamy_mft_rules", ".\\rules\\"]).unwrap();
        let alias: Cli = figue::from_slice(&["mv", r".\a.teamy_mft_rules", ".\\rules\\"]).unwrap();

        assert_eq!(alias, canonical);
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
    fn sync_accepts_target_path() {
        let cli: Cli = figue::from_slice(&["sync", r".\filters.teamy_mft_rules"]).unwrap();

        let Command::Sync(args) = cli.command else {
            panic!("expected sync command");
        };
        assert_eq!(
            args.plan.path.as_deref(),
            Some(r".\filters.teamy_mft_rules")
        );
    }

    #[test]
    fn sync_accepts_recursive_target_path() {
        let cli: Cli =
            figue::from_slice(&["sync", "--recursive", r".\filters.teamy_mft_rules"]).unwrap();

        let Command::Sync(args) = cli.command else {
            panic!("expected sync command");
        };
        assert!(args.plan.recursive);
        assert_eq!(
            args.plan.path.as_deref(),
            Some(r".\filters.teamy_mft_rules")
        );
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
    fn query_accepts_repeated_in_scopes() {
        let cli: Cli =
            figue::from_slice(&["query", "flowers", "--in", r".\src", "--in", r".\tests"]).unwrap();

        let Command::Query(args) = cli.command else {
            panic!("expected query command");
        };
        assert_eq!(
            args.plan.r#in,
            vec![String::from(r".\src"), String::from(r".\tests")]
        );
    }

    #[test]
    fn profile_accepts_profiles_alias() {
        let canonical: Cli = figue::from_slice(&["profile", "list"]).unwrap();
        let alias: Cli = figue::from_slice(&["profiles", "list"]).unwrap();

        assert_eq!(alias, canonical);
    }

    #[test]
    fn profile_tutorial_parses() {
        let cli: Cli = figue::from_slice(&["profile", "tutorial"]).unwrap();

        let Command::Profile(args) = cli.command else {
            panic!("expected profile command");
        };
        assert!(matches!(
            args.command,
            crate::cli::command::profile::ProfileCommand::Tutorial(_)
        ));
    }
}
