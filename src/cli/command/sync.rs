use arbitrary::Arbitrary;
use clap::Args;
use tracing::info;

use crate::windows::win_elevation::ensure_elevated;

#[derive(Args, Arbitrary, PartialEq, Debug, Default)]
pub struct SyncArgs {}

impl SyncArgs {
    pub fn invoke(self) -> eyre::Result<()> {
        ensure_elevated()?;
        info!("sync: TODO - not yet implemented");
        todo!("sync handler not yet implemented")
    }
}

impl crate::cli::to_args::ToArgs for SyncArgs {}
