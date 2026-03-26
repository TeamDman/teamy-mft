pub mod command;
pub mod global_args;
use crate::cli::command::Command;
use crate::cli::global_args::GlobalArgs;
use arbitrary::Arbitrary;
use facet::Facet;
use figue::FigueBuiltins;
use figue::{self as args};

/// Teamy MFT command-line interface.
///
/// Environment variables:
/// - `TEAMY_MFT_SYNC_DIR`: override the persisted sync directory for commands that read cached data.
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
