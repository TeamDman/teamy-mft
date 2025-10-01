#![cfg(debug_assertions)]

use crate::cli::to_args::ToArgs;
use crate::engine::construction::AppConstructionExt;
use crate::engine::scenarios::test_write_bytes_to_file::test_write_bytes_to_file;
use arbitrary::Arbitrary;
use bevy::app::App;
use clap::Args;
use clap::Subcommand;

#[derive(Args, Arbitrary, PartialEq, Debug)]
pub struct TestArgs {
    #[clap(subcommand)]
    pub command: TestCommand,
}

#[derive(Subcommand, Arbitrary, PartialEq, Debug)]
pub enum TestCommand {
    WriteBytesToFile,
}

impl TestArgs {
    pub fn invoke(self) -> eyre::Result<()> {
        match self.command {
            TestCommand::WriteBytesToFile => {
                test_write_bytes_to_file(App::new_headed()?)?;
                Ok(())
            }
        }
    }
}

impl ToArgs for TestArgs {
    fn to_args(&self) -> Vec<std::ffi::OsString> {
        match &self.command {
            TestCommand::WriteBytesToFile => vec!["run".into()],
        }
    }
}
