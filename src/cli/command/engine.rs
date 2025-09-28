use crate::cli::to_args::ToArgs;
use arbitrary::Arbitrary;
use clap::Args;
use clap::Subcommand;

#[derive(Args, Arbitrary, PartialEq, Debug)]
pub struct EngineArgs {
    #[clap(subcommand)]
    pub command: EngineCommand,
}

#[derive(Subcommand, Arbitrary, PartialEq, Debug)]
pub enum EngineCommand {
    Run,
}

impl EngineArgs {
    pub fn invoke(self) -> eyre::Result<()> {
        match self.command {
            EngineCommand::Run => {
                println!("Engine command is a work in progress.");
                Ok(())
            }
        }
    }
}

impl ToArgs for EngineArgs {
    fn to_args(&self) -> Vec<std::ffi::OsString> {
        match &self.command {
            EngineCommand::Run => vec!["run".into()],
        }
    }
}
