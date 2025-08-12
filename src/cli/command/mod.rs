pub mod sync;

use crate::cli::command::sync::SyncArgs;
use crate::cli::to_args::ToArgs;
use arbitrary::Arbitrary;
use clap::Subcommand;
use std::ffi::OsString;
use std::path::PathBuf;
use tracing::info;

/// Teamy MFT commands
#[derive(Subcommand, Arbitrary, PartialEq, Debug)]
pub enum Command {
    /// Sync operations (requires elevation)
    Sync(SyncArgs),
    /// Get the currently configured sync directory
    GetSyncDir,
    /// Set the sync directory (defaults to current directory if omitted)
    SetSyncDir { path: Option<PathBuf> },
}

impl Command {
    pub fn invoke(self) -> eyre::Result<()> {
        match self {
            Command::Sync(args) => args.invoke(),
            Command::GetSyncDir => {
                match crate::sync_dir::get_sync_dir()? {
                    Some(p) => println!("{}", p.display()),
                    None => println!("<not set>"),
                }
                Ok(())
            }
            Command::SetSyncDir { path } => {
                let target = if let Some(p) = path {
                    dunce::canonicalize(p)?
                } else {
                    dunce::canonicalize(std::env::current_dir()?)?
                };
                info!("Setting sync dir to {}", target.display());
                crate::sync_dir::set_sync_dir(target.clone())?;
                println!("Set sync dir to {}", target.display());
                Ok(())
            }
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
            Command::GetSyncDir => {
                args.push("get-sync-dir".into());
            }
            Command::SetSyncDir { path } => {
                args.push("set-sync-dir".into());
                if let Some(p) = path {
                    args.push(p.into());
                }
            }
        }
        args
    }
}
