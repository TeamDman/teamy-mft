use crate::cli::command::run::RunCommand;
use crate::cli::global_args::GlobalArgs;
use crate::cli::to_args::ToArgs;
use arbitrary::Arbitrary;
use clap::Args;
use std::ffi::OsString;

#[derive(Args, Arbitrary, PartialEq, Debug)]
pub struct RunArgs {
    #[command(subcommand)]
    pub command: RunCommand,
}

impl RunArgs {
    pub fn should_init_tracing(&self) -> bool {
        !matches!(self.command, RunCommand::Ui(_))
    }

    pub fn invoke(self, global_args: GlobalArgs) -> eyre::Result<()> {
        let mut global_args = Some(global_args);
        match self.command {
            RunCommand::Ui(args) => args.invoke(global_args.take().unwrap()),
            RunCommand::FileContentsRoundtrip(args) => args.invoke(global_args.take().unwrap()),
            RunCommand::LoadCachedMftFiles(args) => args.invoke(global_args.take().unwrap()),
            RunCommand::Timeout(args) => args.invoke(global_args.take().unwrap()),
        }
    }
}

impl ToArgs for RunArgs {
    fn to_args(&self) -> Vec<OsString> {
        self.command.to_args()
    }
}
