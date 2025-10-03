#![cfg(debug_assertions)]

use crate::cli::to_args::ToArgs;
use crate::engine::construction::AppConstructionExt;
use crate::engine::construction::Testing;
use crate::engine::scenarios::test_predicate_file_extension::test_predicate_file_extension;
use crate::engine::scenarios::test_predicate_path_exists::run_path_exists_predicate_test;
use crate::engine::scenarios::test_predicate_string_ends_with::test_predicate_string_ends_with;
use crate::engine::scenarios::test_timeout::test_timeout;
use crate::engine::scenarios::test_write_bytes_to_file::test_write_bytes_to_file;
use crate::engine::timeout_plugin::ExitTimerJustLog;
use arbitrary::Arbitrary;
use bevy::app::App;
use clap::Args;
use clap::Subcommand;
use std::time::Duration;

#[derive(Args, Arbitrary, PartialEq, Debug)]
pub struct TestArgs {
    #[arg(long, value_parser = humantime::parse_duration)]
    pub timeout: Option<Duration>,
    #[arg(long, default_value = "false")]
    pub headless: bool,
    #[arg(long, default_value = "false")]
    pub keep_open: bool,
    #[clap(subcommand)]
    pub command: TestCommand,
}

#[derive(Subcommand, Arbitrary, PartialEq, Debug)]
pub enum TestCommand {
    WriteBytesToFile,
    Timeout,
    StringEndsWith,
    FileExtension,
    PathExists,
}

impl TestArgs {
    pub fn invoke(self) -> eyre::Result<()> {
        let mut app = if self.headless {
            App::new_headless()?
        } else {
            App::new_headed()?
        };
        app.insert_resource(Testing);
        
        if self.keep_open {
            app.insert_resource(ExitTimerJustLog);
        }
        
        match self.command {
            TestCommand::WriteBytesToFile => {
                test_write_bytes_to_file(app, self.timeout)?;
                Ok(())
            }
            TestCommand::Timeout => {
                test_timeout(app, self.timeout)?;
                Ok(())
            }
            TestCommand::StringEndsWith => {
                test_predicate_string_ends_with(app, self.timeout)?;
                Ok(())
            }
            TestCommand::FileExtension => {
                test_predicate_file_extension(app, self.timeout)?;
                Ok(())
            }
            TestCommand::PathExists => {
                run_path_exists_predicate_test(app, self.timeout)?;
                Ok(())
            }
        }
    }
}

impl ToArgs for TestArgs {
    fn to_args(&self) -> Vec<std::ffi::OsString> {
        match &self.command {
            TestCommand::WriteBytesToFile => vec!["run"],
            TestCommand::Timeout => vec!["timeout"],
            TestCommand::StringEndsWith => vec!["string-ends-with"],
            TestCommand::FileExtension => vec!["file-extension"],
            TestCommand::PathExists => vec!["path-exists"],
        }
        .into_iter()
        .map(Into::into)
        .collect()
    }
}
