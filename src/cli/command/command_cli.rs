use crate::cli::command::get_sync_dir::GetSyncDirArgs;
use crate::cli::command::list_paths::ListPathsArgs;
use crate::cli::command::query::QueryArgs;
use crate::cli::command::set_sync_dir::SetSyncDirArgs;
use crate::cli::command::sync::SyncArgs;
use arbitrary::Arbitrary;
use facet::Facet;

/// Teamy MFT commands
// cli[impl command.surface.core]
// tool[impl cli.help.describes-behavior]
#[derive(Facet, Arbitrary, PartialEq, Debug)]
#[repr(u8)]
pub enum Command {
    /// Write .mft and .mft_search_index files (will auto-elevate via UAC if not already running as administrator)
    Sync(SyncArgs),
    /// Produce newline-delimited list of file paths for matching drives from cached .mft files
    ListPaths(ListPathsArgs),
    /// Get the currently configured sync directory
    GetSyncDir(GetSyncDirArgs),
    /// Set the sync directory (defaults to current directory if omitted)
    SetSyncDir(SetSyncDirArgs),
    /// Query indexed file paths (substring match) across cached `.mft_search_index` files
    Query(QueryArgs),
}

impl Default for Command {
    fn default() -> Self {
        Command::GetSyncDir(GetSyncDirArgs)
    }
}

impl Command {
    /// Invoke the command with global arguments.
    ///
    /// # Errors
    ///
    /// Returns an error if tracing initialization fails or the command execution fails.
    pub fn invoke(self) -> eyre::Result<()> {
        match self {
            Command::Sync(args) => args.invoke(),
            Command::ListPaths(args) => args.invoke(),
            Command::GetSyncDir(args) => args.invoke(),
            Command::SetSyncDir(args) => args.invoke(),
            Command::Query(args) => args.invoke(),
        }
    }
}
