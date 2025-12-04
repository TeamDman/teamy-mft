use crate::cli::command::run::file_contents_roundtrip::FileContentsRoundtripArgs;
use crate::cli::command::run::load_cached_mft_files::LoadCachedMftFilesArgs;
use crate::cli::command::run::orbit_demo::OrbitDemoArgs;
use crate::cli::command::run::timeout::TimeoutArgs;
use crate::cli::command::run::ui::UiArgs;
use crate::cli::to_args::ToArgs;
use arbitrary::Arbitrary;
use clap::Subcommand;
use std::ffi::OsString;

#[derive(Subcommand, Arbitrary, PartialEq, Debug)]
pub enum RunCommand {
    /// Launch the Bevy-powered UI
    Ui(UiArgs),
    /// Preview the orbit camera mechanic
    OrbitDemo(OrbitDemoArgs),
    /// Run the file-contents roundtrip situation
    FileContentsRoundtrip(FileContentsRoundtripArgs),
    /// Load cached MFT files situation
    LoadCachedMftFiles(LoadCachedMftFilesArgs),
    /// Trigger the timeout situation
    Timeout(TimeoutArgs),
}

impl ToArgs for RunCommand {
    fn to_args(&self) -> Vec<OsString> {
        match self {
            RunCommand::Ui(args) => {
                let mut argv = vec!["ui".into()];
                argv.extend(args.to_args());
                argv
            }
            RunCommand::OrbitDemo(args) => {
                let mut argv = vec!["orbit-demo".into()];
                argv.extend(args.to_args());
                argv
            }
            RunCommand::FileContentsRoundtrip(args) => {
                let mut argv = vec!["file-contents-roundtrip".into()];
                argv.extend(args.to_args());
                argv
            }
            RunCommand::LoadCachedMftFiles(args) => {
                let mut argv = vec!["load-cached-mft-files".into()];
                argv.extend(args.to_args());
                argv
            }
            RunCommand::Timeout(args) => {
                let mut argv = vec!["timeout".into()];
                argv.extend(args.to_args());
                argv
            }
        }
    }
}
