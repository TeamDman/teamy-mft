use crate::cli::to_args::ToArgs;
use crate::engine::scenarios::test_timeout::test_timeout;
use arbitrary::Arbitrary;
use bevy::app::App;
use clap::Args;
use std::ffi::OsString;
use std::time::Duration;

#[derive(Args, Arbitrary, PartialEq, Debug, Default)]
pub struct TimeoutArgs;

impl TimeoutArgs {
    pub fn invoke(self, app: App, timeout: Option<Duration>) -> eyre::Result<()> {
        test_timeout(app, timeout)
    }
}

impl ToArgs for TimeoutArgs {
    fn to_args(&self) -> Vec<OsString> {
        Vec::new()
    }
}
