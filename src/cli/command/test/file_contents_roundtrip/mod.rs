use crate::cli::to_args::ToArgs;
use crate::engine::scenarios::test_file_contents_roundtrip::test_file_contents_roundtrip;
use arbitrary::Arbitrary;
use bevy::app::App;
use clap::Args;
use std::ffi::OsString;
use std::time::Duration;

#[derive(Args, Arbitrary, PartialEq, Debug, Default)]
pub struct FileContentsRoundtripArgs;

impl FileContentsRoundtripArgs {
    pub fn invoke(self, app: App, timeout: Option<Duration>) -> eyre::Result<()> {
        test_file_contents_roundtrip(app, timeout)
    }
}

impl ToArgs for FileContentsRoundtripArgs {
    fn to_args(&self) -> Vec<OsString> {
        Vec::new()
    }
}
