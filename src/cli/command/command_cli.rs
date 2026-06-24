use crate::cancellation::CancellationToken;
use crate::cli::command::fsutil::FsutilArgs;
use crate::cli::command::install::InstallArgs;
use crate::cli::command::list_paths::ListPathsArgs;
use crate::cli::command::r#move::MoveArgs;
use crate::cli::command::profile::ProfileArgs;
use crate::cli::command::protection::ProtectionArgs;
use crate::cli::command::query::QueryArgs;
use crate::cli::command::rules::RuleArgs;
use crate::cli::command::service::ServiceArgs;
use crate::cli::command::status::StatusArgs;
use crate::cli::command::sync::SyncArgs;
use crate::cli::command::tray::TrayArgs;
use crate::cli::command::uninstall::UninstallArgs;
use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};

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
    /// Configure the machine-wide cache and optionally install the daemon service
    Install(InstallArgs),
    /// Helper for `service uninstall`
    Uninstall(UninstallArgs),
    /// Produce newline-delimited list of file paths for matching drives from cached `.mft` files
    ListPaths(ListPathsArgs),
    /// Move one file and automatically refresh the published overlay for the old and new paths
    #[facet(args::alias = "mv")]
    Move(MoveArgs),
    /// Manage discovered `.teamy_mft_rules` profile rules used to filter query results
    #[facet(args::alias = "rules")]
    Rule(RuleArgs),
    /// Manage query rule profiles discovered from `.teamy_mft_rules` files
    #[facet(args::alias = "profiles")]
    Profile(ProfileArgs),
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
    pub fn invoke(self, cancellation_token: CancellationToken) -> eyre::Result<()> {
        match self {
            Command::Daemon(args) | Command::Service(args) => args.invoke(cancellation_token),
            Command::Sync(args) => args.invoke(cancellation_token),
            Command::Install(args) => args.invoke(),
            Command::Uninstall(args) => args.invoke(),
            Command::ListPaths(args) => args.invoke(cancellation_token),
            Command::Move(args) => args.invoke(),
            Command::Rule(args) => args.invoke(),
            Command::Profile(args) => args.invoke(),
            Command::Protection(args) => args.invoke(),
            Command::Fsutil(args) => args.invoke(),
            Command::Status(args) => args.invoke(),
            Command::Query(args) => args.invoke_and_print(cancellation_token),
            Command::Tray(args) => args.invoke(),
        }
    }
}
