#![cfg(debug_assertions)]

use crate::cli::global_args::GlobalArgs;
use crate::cli::to_args::ToArgs;
use crate::engine::construction::AppConstructionExt;
use crate::engine::construction::Testing;
use crate::engine::scenarios::test_file_contents_roundtrip::test_file_contents_roundtrip;
use crate::engine::scenarios::test_load_cached_mft_files::test_load_cached_mft_files;
use crate::engine::scenarios::test_timeout::test_timeout;
use crate::engine::timeout_plugin::KeepOpen;
use arbitrary::Arbitrary;
use bevy::app::App;
use clap::Args;
use clap::Subcommand;
use std::time::Duration;

#[derive(Args, Arbitrary, PartialEq, Debug)]
pub struct TestArgs {
    #[arg(long, value_parser = humantime::parse_duration, global = true)]
    pub timeout: Option<Duration>,
    #[arg(long, default_value = "false", global = true)]
    pub headless: bool,
    #[arg(long, default_value = "false", global = true)]
    pub keep_open: bool,
    #[clap(subcommand)]
    pub command: TestCommand,
}

#[derive(Subcommand, Arbitrary, PartialEq, Debug)]
pub enum TestCommand {
    FileContentsRoundtrip,
    LoadCachedMftFiles,
    Timeout,
}

impl TestArgs {
    pub fn invoke(self, global_args: GlobalArgs) -> eyre::Result<()> {
        let mut app = if self.headless {
            App::new_headless()?
        } else {
            App::new_headed(global_args)?
        };
        app.insert_resource(Testing);

        if self.keep_open {
            app.insert_resource(KeepOpen);
        }

        match self.command {
            TestCommand::FileContentsRoundtrip => {
                test_file_contents_roundtrip(app, self.timeout)?;
                Ok(())
            }
            TestCommand::LoadCachedMftFiles => {
                test_load_cached_mft_files(app, self.timeout)?;
                Ok(())
            }
            TestCommand::Timeout => {
                test_timeout(app, self.timeout)?;
                Ok(())
            }
        }
    }
}

impl ToArgs for TestArgs {
    fn to_args(&self) -> Vec<std::ffi::OsString> {
        match &self.command {
            TestCommand::FileContentsRoundtrip => vec!["file-contents-roundtrip"],
            TestCommand::LoadCachedMftFiles => vec!["load-cached-mft-files"],
            TestCommand::Timeout => vec!["timeout"],
        }
        .into_iter()
        .map(Into::into)
        .collect()
    }
}
