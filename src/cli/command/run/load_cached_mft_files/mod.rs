use crate::cli::to_args::ToArgs;
use crate::engine::scenarios::test_load_cached_mft_files::test_load_cached_mft_files;
use arbitrary::Arbitrary;
use bevy::app::App;
use clap::Args;
use std::ffi::OsString;
use std::time::Duration;

#[derive(Args, Arbitrary, PartialEq, Debug, Default)]
pub struct LoadCachedMftFilesArgs;

impl LoadCachedMftFilesArgs {
    pub fn invoke(self, app: App, timeout: Option<Duration>) -> eyre::Result<()> {
        test_load_cached_mft_files(app, timeout)
    }
}

impl ToArgs for LoadCachedMftFilesArgs {
    fn to_args(&self) -> Vec<OsString> {
        Vec::new()
    }
}
