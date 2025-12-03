use super::load_cached_mft_files_situation::load_cached_mft_files_situation;
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

#[derive(Args, Arbitrary, PartialEq, Debug, Default)]
pub struct LoadCachedMftFilesArgs {
    #[arg(long, value_parser = humantime::parse_duration)]
    pub timeout: Option<Duration>,
    #[arg(long, default_value_t = false)]
    pub headless: bool,
    #[arg(long, default_value_t = false)]
    pub keep_open: bool,
}

impl LoadCachedMftFilesArgs {
    pub fn invoke(self, global_args: GlobalArgs) -> eyre::Result<()> {
        let app = self.build_app(global_args)?;
        load_cached_mft_files_situation(app, self.timeout)
    }

    fn build_app(&self, global_args: GlobalArgs) -> eyre::Result<App> {
        let mut app = if self.headless {
            App::new_headless()?
        } else {
            App::new_headed(global_args)?
        };
        app.insert_resource(Testing);

        if self.keep_open {
            app.insert_resource(KeepOpen);
        }

        Ok(app)
    }
}

impl ToArgs for LoadCachedMftFilesArgs {
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
        args
    }
}
