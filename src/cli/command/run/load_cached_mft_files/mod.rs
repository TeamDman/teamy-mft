mod load_cached_mft_files_situation;

use crate::cli::to_args::ToArgs;
use arbitrary::Arbitrary;
use bevy::app::App;
use clap::Args;
use load_cached_mft_files_situation::load_cached_mft_files_situation;
use std::ffi::OsString;
use std::time::Duration;

#[derive(Args, Arbitrary, PartialEq, Debug, Default)]
pub struct LoadCachedMftFilesArgs;

impl LoadCachedMftFilesArgs {
    pub fn invoke(self, app: App, timeout: Option<Duration>) -> eyre::Result<()> {
        load_cached_mft_files_situation(app, timeout)
    }
}

impl ToArgs for LoadCachedMftFilesArgs {
    fn to_args(&self) -> Vec<OsString> {
        Vec::new()
    }
}
