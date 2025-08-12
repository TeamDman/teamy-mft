use crate::sync_dir::try_get_sync_dir;
use crate::windows::win_elevation::ensure_elevated;
use arbitrary::Arbitrary;
use clap::Args;
use tracing::info;

#[derive(Args, Arbitrary, PartialEq, Debug, Default)]
pub struct SyncArgs {}

impl SyncArgs {
    pub fn invoke(self) -> eyre::Result<()> {
        let sync_dir = try_get_sync_dir()?;
        ensure_elevated()?;
        info!("Syncing to directory: {}", sync_dir.display());
        todo!("sync handler not yet implemented")
    }
}

impl crate::cli::to_args::ToArgs for SyncArgs {}
