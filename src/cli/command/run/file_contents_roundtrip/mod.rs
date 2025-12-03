mod file_contents_roundtrip_situation;

use crate::cli::to_args::ToArgs;
use arbitrary::Arbitrary;
use bevy::app::App;
use clap::Args;
use file_contents_roundtrip_situation::file_contents_roundtrip_situation;
use std::ffi::OsString;
use std::time::Duration;

#[derive(Args, Arbitrary, PartialEq, Debug, Default)]
pub struct FileContentsRoundtripArgs;

impl FileContentsRoundtripArgs {
    pub fn invoke(self, app: App, timeout: Option<Duration>) -> eyre::Result<()> {
        file_contents_roundtrip_situation(app, timeout)
    }
}

impl ToArgs for FileContentsRoundtripArgs {
    fn to_args(&self) -> Vec<OsString> {
        Vec::new()
    }
}
