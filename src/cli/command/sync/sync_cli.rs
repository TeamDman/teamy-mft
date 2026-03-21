use std::collections::BTreeMap;

use crate::cli::command::sync::IfExistsOutputBehaviour;
use crate::cli::command::sync::drive_sync_info::DriveSyncInfo;
use crate::cli::command::sync::drive_sync_info::resolve_drive_infos;
use crate::cli::command::sync::index::SyncIndexArgs;
use crate::cli::command::sync::mft::SyncMftArgs;
use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};
use futures::TryStreamExt;
use teamy_windows::storage::DriveLetterPattern;
use tokio_stream::StreamExt;
use tracing::Instrument;
use tracing::info_span;

#[derive(Facet, PartialEq, Debug, Arbitrary, Default)]
pub struct SyncArgs {
    /// Drive letter pattern to match drives to sync (e.g., "*", "C", "CD", "C,D")
    #[facet(args::named, default)]
    pub drive_letter_pattern: DriveLetterPattern,

    /// How to handle existing output files
    #[facet(args::named, default)]
    pub if_exists: IfExistsOutputBehaviour,

    /// Sync stage to run
    #[facet(args::subcommand)]
    pub command: Option<SyncCommand>,
}

impl SyncArgs {
    /// Sync MFT data from drives.
    ///
    /// # Errors
    ///
    /// Returns an error if the sync directory cannot be retrieved, elevation fails,
    /// or if reading/writing MFT data fails.
    pub fn invoke(self) -> eyre::Result<()> {
        let drive_infos = resolve_drive_infos(&self.drive_letter_pattern)?;
        let if_exists = &self.if_exists;

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;

        runtime.block_on(async move {
            let command = self.command.unwrap_or_default();
            let drive_infos = command.invoke_preflight(drive_infos, if_exists)?;
            command.invoke(drive_infos).await?;
            Ok(())
        })
    }
}

#[derive(Facet, Arbitrary, PartialEq, Debug, Clone, Default)]
#[repr(u8)]
#[facet(rename_all = "kebab-case")]
pub enum SyncCommand {
    /// Sync raw .mft snapshots
    Mft(SyncMftArgs),
    /// Build `.mft_search_index` files from snapshots
    Index(SyncIndexArgs),
    /// Sync both stages sequentially, with preflight checks and error handling for both stages
    #[default]
    Both,
}

impl SyncCommand {
    /// Validate the sync can proceed before any command-specific work begins.
    ///
    /// # Errors
    ///
    /// Returns an error if preflight validation fails.
    pub fn invoke_preflight(
        &self,
        drive_infos: BTreeMap<char, DriveSyncInfo>,
        if_exists: &IfExistsOutputBehaviour,
    ) -> eyre::Result<BTreeMap<char, DriveSyncInfo>> {
        match self {
            Self::Mft(SyncMftArgs) => SyncMftArgs.invoke_preflight(drive_infos, if_exists),
            Self::Index(SyncIndexArgs) => SyncIndexArgs.invoke_preflight(drive_infos, if_exists),
            Self::Both => {
                SyncMftArgs.invoke_preflight(drive_infos, if_exists)
            }
        }
    }

    /// # Errors
    ///
    /// Returns an error if the sync fails, likely caused by IO problems.
    pub async fn invoke(&self, drive_infos: BTreeMap<char, DriveSyncInfo>) -> eyre::Result<()> {
        match self {
            Self::Mft(SyncMftArgs) => {
                let mft_data = SyncMftArgs
                    .invoke(drive_infos)
                    .instrument(info_span!("dispatch mft sync work"))
                    .await?;
                tokio::pin!(mft_data);
                let _guard = info_span!("collect mft sync results").entered();
                while let Some(result) = mft_data.next().await {
                    result?;
                }
                Ok(())
            }
            Self::Index(SyncIndexArgs) => SyncIndexArgs.invoke(drive_infos).await,
            Self::Both => {
                let mft_data = SyncMftArgs
                    .invoke(drive_infos)
                    .instrument(info_span!("dispatch mft sync work"))
                    .await?.map_ok(async |(drive_info, physical_mft)| {
                        
                    });
                Ok(())
            }
        }
    }
}
