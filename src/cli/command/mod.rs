pub mod check;
pub mod engine;
pub mod get_sync_dir;
pub mod list_paths;
pub mod query;
pub mod robocopy_logs_tui;
pub mod set_sync_dir;
pub mod sync;
pub mod test;

use crate::cli::command::check::CheckArgs;
use crate::cli::command::engine::EngineArgs;
use crate::cli::command::get_sync_dir::GetSyncDirArgs;
use crate::cli::command::list_paths::ListPathsArgs;
use crate::cli::command::query::QueryArgs;
use crate::cli::command::robocopy_logs_tui::RobocopyLogsTuiArgs;
use crate::cli::command::set_sync_dir::SetSyncDirArgs;
use crate::cli::command::sync::SyncArgs;
#[cfg(debug_assertions)]
use crate::cli::command::test::TestArgs;
use crate::cli::global_args::GlobalArgs;
use crate::cli::to_args::ToArgs;
use crate::init_tracing;
use arbitrary::Arbitrary;
use clap::Subcommand;
use std::ffi::OsString;

/// Teamy MFT commands
#[derive(Subcommand, Arbitrary, PartialEq, Debug)]
pub enum Command {
    /// Sync operations (requires elevation)
    Sync(SyncArgs),
    /// Produce newline-delimited list of file paths for matching drives from cached .mft files
    ListPaths(ListPathsArgs),
    /// Get the currently configured sync directory
    GetSyncDir(GetSyncDirArgs),
    /// Set the sync directory (defaults to current directory if omitted)
    SetSyncDir(SetSyncDirArgs),
    /// Validate cached MFT files have at least one Win32 FILE_NAME attribute per entry having any FILE_NAME
    Check(CheckArgs),
    /// Query resolved file paths (substring match) across cached MFTs
    Query(QueryArgs),
    /// Explore robocopy logs in a TUI (validate file exists for now)
    RobocopyLogsTui(RobocopyLogsTuiArgs),
    /// Run the Bevy game engine to visualize MFT data processing
    Engine(EngineArgs),
    /// Test commands (only available in debug builds)
    #[cfg(debug_assertions)]
    Test(TestArgs),
}

impl Default for Command {
    fn default() -> Self {
        Command::GetSyncDir(GetSyncDirArgs)
    }
}

impl Command {
    pub fn invoke(self, global_args: GlobalArgs) -> eyre::Result<()> {
        match self {
            Command::Engine(_) => {}
            _ => {
                init_tracing(global_args.log_level());
            }
        }
        match self {
            Command::Sync(args) => args.invoke(),
            Command::ListPaths(args) => args.invoke(),
            Command::GetSyncDir(args) => args.invoke(),
            Command::SetSyncDir(args) => args.invoke(),
            Command::Check(args) => args.invoke(),
            Command::Query(args) => args.invoke(),
            Command::RobocopyLogsTui(args) => args.invoke(),
            Command::Engine(args) => args.invoke(),
            #[cfg(debug_assertions)]
            Command::Test(args) => args.invoke(),
        }
    }
}

impl ToArgs for Command {
    fn to_args(&self) -> Vec<OsString> {
        let mut args = Vec::new();
        match self {
            Command::Sync(sync_args) => {
                args.push("sync".into());
                args.extend(sync_args.to_args());
            }
            Command::ListPaths(list_paths_args) => {
                args.push("list-paths".into());
                args.extend(list_paths_args.to_args());
            }
            Command::GetSyncDir(get_args) => {
                args.push("get-sync-dir".into());
                args.extend(get_args.to_args());
            }
            Command::SetSyncDir(set_args) => {
                args.push("set-sync-dir".into());
                args.extend(set_args.to_args());
            }
            Command::Check(check_args) => {
                args.push("check".into());
                args.extend(check_args.to_args());
            }
            Command::Query(query_args) => {
                args.push("query".into());
                args.extend(query_args.to_args());
            }
            Command::RobocopyLogsTui(logs_args) => {
                args.push("robocopy-logs-tui".into());
                args.extend(logs_args.to_args());
            }
            Command::Engine(engine_args) => {
                args.push("engine".into());
                args.extend(engine_args.to_args());
            }
            #[cfg(debug_assertions)]
            Command::Test(test_args) => {
                args.push("test".into());
                args.extend(test_args.to_args());
            }
        }
        args
    }
}
