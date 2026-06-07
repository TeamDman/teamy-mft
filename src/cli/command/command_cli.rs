use crate::cli::command::fsutil::FsutilArgs;
use crate::cli::command::install::InstallArgs;
use crate::cli::command::list_paths::ListPathsArgs;
use crate::cli::command::profile::ProfileArgs;
use crate::cli::command::protection::ProtectionArgs;
use crate::cli::command::query::QueryArgs;
use crate::cli::command::rules::RulesArgs;
use crate::cli::command::service::ServiceArgs;
use crate::cli::command::status::StatusArgs;
use crate::cli::command::sync::SyncArgs;
use crate::cli::command::tray::TrayArgs;
use crate::cli::command::uninstall::UninstallArgs;
use arbitrary::Arbitrary;
use facet::Facet;

/// Teamy MFT commands
// cli[impl command.surface.core]
// tool[impl cli.help.describes-behavior]
#[derive(Facet, Arbitrary, PartialEq, Debug)]
#[repr(u8)]
pub enum Command {
    /// Compatibility alias for `service`
    Daemon(ServiceArgs),
    /// Manage the machine-wide Windows service that hosts the daemon
    Service(ServiceArgs),
    /// Write `.mft` and `.mft_search_index` files (will auto-elevate via UAC if not already running as administrator)
    Sync(SyncArgs),
    /// Helper for `service install`
    Install(InstallArgs),
    /// Helper for `service uninstall`
    Uninstall(UninstallArgs),
    /// Produce newline-delimited list of file paths for matching drives from cached `.mft` files
    ListPaths(ListPathsArgs),
    /// Manage `.teamy_mft_rules` profile rules used to filter query results
    Rules(RulesArgs),
    /// Manage query rule profiles discovered from `.teamy_mft_rules` files
    Profile(ProfileArgs),
    /// Compatibility alias for `profile`
    Profiles(ProfileArgs),
    /// Toggle machine cache ACL protection for development workflows
    Protection(ProtectionArgs),
    /// Native Windows filesystem utilities used by teamy-mft
    Fsutil(FsutilArgs),
    /// Show per-drive cache freshness for `.mft` and `.mft_search_index` files
    Status(StatusArgs),
    /// Query indexed file paths (substring match) across cached `.mft_search_index` files
    Query(QueryArgs),
    /// Launch the Windows tray icon for daemon log replay and live follow
    Tray(TrayArgs),
}

impl Default for Command {
    fn default() -> Self {
        Command::Status(StatusArgs::default())
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
            Command::Daemon(args) | Command::Service(args) => args.invoke(),
            Command::Sync(args) => args.invoke(),
            Command::Install(args) => args.invoke(),
            Command::Uninstall(args) => args.invoke(),
            Command::ListPaths(args) => args.invoke(),
            Command::Rules(args) => args.invoke(),
            Command::Profile(args) | Command::Profiles(args) => args.invoke(),
            Command::Protection(args) => args.invoke(),
            Command::Fsutil(args) => args.invoke(),
            Command::Status(args) => args.invoke(),
            Command::Query(args) => args.invoke_and_print(),
            Command::Tray(args) => args.invoke(),
        }
    }
}
