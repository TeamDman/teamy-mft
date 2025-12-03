mod timeout_situation;

use crate::cli::to_args::ToArgs;
use arbitrary::Arbitrary;
use bevy::app::App;
use clap::Args;
use std::ffi::OsString;
use std::time::Duration;
use timeout_situation::timeout_situation;

#[derive(Args, Arbitrary, PartialEq, Debug, Default)]
pub struct TimeoutArgs;

impl TimeoutArgs {
    pub fn invoke(self, app: App, timeout: Option<Duration>) -> eyre::Result<()> {
        timeout_situation(app, timeout)
    }
}

impl ToArgs for TimeoutArgs {
    fn to_args(&self) -> Vec<OsString> {
        Vec::new()
    }
}
