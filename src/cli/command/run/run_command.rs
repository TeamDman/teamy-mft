use crate::cli::command::test::file_contents_roundtrip::FileContentsRoundtripArgs;
use crate::cli::command::test::load_cached_mft_files::LoadCachedMftFilesArgs;
use crate::cli::command::test::timeout::TimeoutArgs;
use crate::cli::to_args::ToArgs;
use arbitrary::Arbitrary;
use bevy::app::App;
use clap::Subcommand;
use std::ffi::OsString;
use std::time::Duration;

#[derive(Subcommand, Arbitrary, PartialEq, Debug)]
pub enum TestCommand {
    FileContentsRoundtrip(FileContentsRoundtripArgs),
    LoadCachedMftFiles(LoadCachedMftFilesArgs),
    Timeout(TimeoutArgs),
}

impl TestCommand {
    pub fn invoke(self, app: App, timeout: Option<Duration>) -> eyre::Result<()> {
        match self {
            TestCommand::FileContentsRoundtrip(args) => args.invoke(app, timeout),
            TestCommand::LoadCachedMftFiles(args) => args.invoke(app, timeout),
            TestCommand::Timeout(args) => args.invoke(app, timeout),
        }
    }
}

impl ToArgs for TestCommand {
    fn to_args(&self) -> Vec<OsString> {
        match self {
            TestCommand::FileContentsRoundtrip(args) => {
                let mut argv = vec!["file-contents-roundtrip".into()];
                argv.extend(args.to_args());
                argv
            }
            TestCommand::LoadCachedMftFiles(args) => {
                let mut argv = vec!["load-cached-mft-files".into()];
                argv.extend(args.to_args());
                argv
            }
            TestCommand::Timeout(args) => {
                let mut argv = vec!["timeout".into()];
                argv.extend(args.to_args());
                argv
            }
        }
    }
}
