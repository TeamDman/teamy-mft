#![cfg(debug_assertions)]

use crate::cli::to_args::ToArgs;
use crate::engine::construction::AppConstructionExt;
use crate::engine::scenarios::test_write_bytes_to_file::test_write_bytes_to_file;
use crate::engine::timeout_plugin::TimeoutExitConfig;
use arbitrary::Arbitrary;
use bevy::app::App;
use clap::Args;
use clap::Subcommand;
use std::time::Duration;

#[derive(Args, Arbitrary, PartialEq, Debug)]
pub struct TestArgs {
    #[arg(long, value_parser = humantime::parse_duration)]
    pub timeout: Option<Duration>,
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
                let mut engine = App::new_headed()?;
                if let Some(timeout) = self.timeout {
                    // Override default timeout if specified
                    engine.insert_resource(TimeoutExitConfig::from(timeout));
                }
                test_write_bytes_to_file(engine)?;
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
