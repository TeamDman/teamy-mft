use crate::cli::command::Command;
use crate::cli::Cli;
use crate::sync_dir::get_sync_dir;
use crate::windows::win_elevation::ensure_elevated;
use arbitrary::Arbitrary;
use clap::Args;
use tracing::info;

#[derive(Args, Arbitrary, PartialEq, Debug, Default)]
pub struct SyncArgs {}

impl SyncArgs {
    pub fn invoke(self) -> eyre::Result<()> {
        let Some(sync_dir) = get_sync_dir()? else {
            eyre::bail!(
                "Sync directory is not set. Please set it using the `{}` command.",
                Cli {
                    command: Command::SetSyncDir { path: None },
                    ..Default::default()
                }
                .display_invocation()
            );
        };
        ensure_elevated()?;
        info!("Syncing to directory: {}", sync_dir.display());
        todo!("sync handler not yet implemented")
    }
}

impl crate::cli::to_args::ToArgs for SyncArgs {}
