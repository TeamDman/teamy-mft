use crate::cli::command::sync::IfExistsOutputBehaviour;
use crate::cli::command::sync::drive_sync_info::DriveSyncInfo;
use crate::cli::command::sync::drive_sync_info::resolve_drive_infos;
use crate::cli::command::sync::index::SyncIndexArgs;
use crate::cli::command::sync::mft::SyncMftArgs;
use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};
use futures::TryStreamExt;
use std::collections::BTreeSet;
use std::sync::Arc;
use teamy_windows::storage::DriveLetterPattern;
use tokio_stream::StreamExt;
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
            self.command
                .unwrap_or_default()
                .invoke(drive_infos, if_exists)
                .await
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
    /// # Errors
    ///
    /// Returns an error if the sync fails, likely caused by IO problems.
    pub async fn invoke(
        &self,
        drive_infos: Vec<DriveSyncInfo>,
        if_exists: &IfExistsOutputBehaviour,
    ) -> eyre::Result<()> {
        match self {
            Self::Mft(SyncMftArgs) => {
                let drive_infos = SyncMftArgs::invoke_preflight(drive_infos, if_exists)?;
                let mft_data = {
                    let _guard = info_span!("dispatch mft sync work").entered();
                    SyncMftArgs::invoke(drive_infos)?
                };
                tokio::pin!(mft_data);
                let _guard = info_span!("collect mft sync results").entered();
                while let Some(result) = mft_data.next().await {
                    result?;
                }
                Ok(())
            }
            Self::Index(SyncIndexArgs) => {
                let drive_infos = SyncIndexArgs::invoke_preflight(drive_infos, if_exists)?;
                SyncIndexArgs.invoke(drive_infos)
            }
            Self::Both => {
                // The two stages have different skip/overwrite/abort filtering rules, so
                // they must each run their own preflight over the same initial drive set.
                let mft_drive_infos =
                    SyncMftArgs::invoke_preflight(drive_infos.clone(), if_exists)?;
                let index_drive_infos = SyncIndexArgs::invoke_preflight(drive_infos, if_exists)?;

                // Drives present in both sets can build the index directly from the fresh
                // in-memory `PhysicalMftReadResult` produced by the MFT stage, avoiding a
                // write-then-read roundtrip through the cached `.mft` file.
                let mft_drive_letters = mft_drive_infos
                    .iter()
                    .map(|info| info.drive_letter)
                    .collect::<BTreeSet<_>>();
                let in_memory_index_drive_letters = Arc::new(
                    index_drive_infos
                        .iter()
                        .map(|info| info.drive_letter)
                        .filter(|drive_letter| mft_drive_letters.contains(drive_letter))
                        .collect::<BTreeSet<_>>(),
                );

                // Any drive that still needs indexing but is not part of the current MFT sync
                // cannot use the in-memory fast path. This happens, for example, when the MFT
                // file already exists and MFT sync is skipped, but the search index still needs
                // to be built from the cached `.mft` on disk.
                let fallback_index_drive_infos = index_drive_infos
                    .into_iter()
                    .filter(|info| !mft_drive_letters.contains(&info.drive_letter))
                    .collect::<Vec<_>>();

                let mft_data = {
                    let _guard = info_span!("dispatch mft sync work").entered();
                    SyncMftArgs::invoke(mft_drive_infos)?
                };

                let in_memory_index_drive_letters_for_stream =
                    Arc::clone(&in_memory_index_drive_letters);
                let in_memory_indexing = async move {
                    // Consume completed MFT reads as they arrive and fan index construction out
                    // concurrently so slow drives do not block faster ones.
                    let _guard = info_span!(
                        "collect mft sync and in_memory index results",
                        drive_count = in_memory_index_drive_letters_for_stream.len(),
                    )
                    .entered();
                    mft_data
                        .try_for_each_concurrent(None, move |(drive_info, physical_mft)| {
                            let in_memory_index_drive_letters =
                                Arc::clone(&in_memory_index_drive_letters_for_stream);
                            async move {
                                if !in_memory_index_drive_letters.contains(&drive_info.drive_letter)
                                {
                                    return Ok(());
                                }

                                tokio::task::spawn_blocking(move || {
                                    let _guard = info_span!(
                                        "build_in_memory_search_index_for_drive",
                                        drive = %drive_info.drive_letter,
                                        index_path = %drive_info.index_output_path.display(),
                                    )
                                    .entered();
                                    let mft_file = physical_mft.to_mft_file()?;
                                    SyncIndexArgs.invoke_for_mft_file(&drive_info, &mft_file)
                                })
                                .await
                                .map_err(|error| {
                                    eyre::eyre!("Failed joining in-memory index task: {error}")
                                })??;

                                Ok(())
                            }
                        })
                        .await
                };

                let disk_indexing = async move {
                    // Run the disk-backed index path in parallel with the in-memory path so
                    // drives skipped by the MFT stage do not have to wait for fresh MFT reads.
                    if fallback_index_drive_infos.is_empty() {
                        return Ok(());
                    }

                    let _guard = info_span!(
                        "build_disk_backed_search_indexes",
                        drive_count = fallback_index_drive_infos.len(),
                    )
                    .entered();
                    SyncIndexArgs.invoke(fallback_index_drive_infos)
                };

                tokio::try_join!(in_memory_indexing, disk_indexing)?;

                Ok(())
            }
        }
    }
}
