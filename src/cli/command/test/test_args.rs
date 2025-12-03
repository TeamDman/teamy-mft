use crate::cli::command::test::TestCommand;
use crate::cli::global_args::GlobalArgs;
use crate::cli::to_args::ToArgs;
use crate::engine::construction::AppConstructionExt;
use crate::engine::construction::Testing;
use crate::engine::timeout_plugin::KeepOpen;
use arbitrary::Arbitrary;
use bevy::app::App;
use clap::Args;
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

        self.command.invoke(app, self.timeout)
    }
}

impl ToArgs for TestArgs {
    fn to_args(&self) -> Vec<std::ffi::OsString> {
        self.command.to_args()
    }
}
