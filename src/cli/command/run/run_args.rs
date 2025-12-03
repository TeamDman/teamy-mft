use crate::cli::command::run::RunCommand;
use crate::cli::global_args::GlobalArgs;
use crate::cli::to_args::ToArgs;
use crate::engine::construction::AppConstructionExt;
use crate::engine::construction::Testing;
use crate::engine::timeout_plugin::KeepOpen;
use arbitrary::Arbitrary;
use bevy::app::App;
use clap::Args;
use std::ffi::OsString;
use std::time::Duration;

#[derive(Args, Arbitrary, PartialEq, Debug)]
pub struct RunArgs {
    #[arg(long, value_parser = humantime::parse_duration, global = true)]
    pub timeout: Option<Duration>,
    #[arg(long, default_value = "false", global = true)]
    pub headless: bool,
    #[arg(long, default_value = "false", global = true)]
    pub keep_open: bool,
    #[command(subcommand)]
    pub command: RunCommand,
}

impl RunArgs {
    pub fn should_init_tracing(&self) -> bool {
        !matches!(self.command, RunCommand::Ui(_))
    }

    pub fn invoke(self, global_args: GlobalArgs) -> eyre::Result<()> {
        let RunArgs {
            timeout,
            headless,
            keep_open,
            command,
        } = self;

        match command {
            RunCommand::Ui(args) => args.invoke(global_args),
            RunCommand::FileContentsRoundtrip(args) => Self::invoke_situation_command(
                global_args,
                headless,
                keep_open,
                timeout,
                |app, timeout| args.invoke(app, timeout),
            ),
            RunCommand::LoadCachedMftFiles(args) => Self::invoke_situation_command(
                global_args,
                headless,
                keep_open,
                timeout,
                |app, timeout| args.invoke(app, timeout),
            ),
            RunCommand::Timeout(args) => Self::invoke_situation_command(
                global_args,
                headless,
                keep_open,
                timeout,
                |app, timeout| args.invoke(app, timeout),
            ),
        }
    }

    fn invoke_situation_command<F>(
        global_args: GlobalArgs,
        headless: bool,
        keep_open: bool,
        timeout: Option<Duration>,
        f: F,
    ) -> eyre::Result<()>
    where
        F: FnOnce(App, Option<Duration>) -> eyre::Result<()>,
    {
        let mut app = if headless {
            App::new_headless()?
        } else {
            App::new_headed(global_args)?
        };
        app.insert_resource(Testing);

        if keep_open {
            app.insert_resource(KeepOpen);
        }

        f(app, timeout)
    }
}

impl ToArgs for RunArgs {
    fn to_args(&self) -> Vec<OsString> {
        let mut args = Vec::new();
        if let Some(timeout) = self.timeout {
            args.push("--timeout".into());
            args.push(humantime::format_duration(timeout).to_string().into());
        }
        if self.headless {
            args.push("--headless".into());
        }
        if self.keep_open {
            args.push("--keep-open".into());
        }
        args.extend(self.command.to_args());
        args
    }
}
