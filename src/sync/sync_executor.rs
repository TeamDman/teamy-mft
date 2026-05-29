use crate::sync::DriveSyncInfo;
use crate::sync::IfExistsOutputBehaviour;
use crate::sync::SyncIndex;
use crate::sync::SyncMft;
use crate::sync::SyncMode;
use futures::TryStreamExt;
use std::collections::BTreeSet;
use std::sync::Arc;
use tokio_stream::StreamExt;
use tracing::Instrument;
use tracing::Span;
use tracing::info_span;

/// # Errors
///
/// Returns an error if the sync fails, likely caused by IO problems.
#[expect(
    clippy::too_many_lines,
    reason = "This function coordinates multiple sync paths and error-handling branches."
)]
pub async fn execute_sync_mode(
    mode: SyncMode,
    drive_infos: Vec<DriveSyncInfo>,
    if_exists: &IfExistsOutputBehaviour,
) -> eyre::Result<()> {
    match mode {
        SyncMode::Mft => {
            let drive_infos = SyncMft::invoke_preflight(drive_infos, if_exists)?;
            let mft_span = info_span!("dispatch mft sync work");
            let mft_data = {
                let _guard = mft_span.enter();
                SyncMft::invoke(drive_infos)?
            };
            tokio::pin!(mft_data);
            tracing::debug!("Collecting MFT sync results");
            while let Some(result) = async { mft_data.next().await }
                .instrument(mft_span.clone())
                .await
            {
                let (drive_info, physical_mft) = result?;
                {
                    let _guard = info_span!(
                        "drop_physical_mft_read_result",
                        drive = %drive_info.drive_letter,
                        physical_segments = physical_mft.physical_read_results.entries.len(),
                        logical_segments = physical_mft.logical_read_plan.segments.len(),
                    )
                    .entered();
                    drop(physical_mft);
                }
            }
            Ok(())
        }
        SyncMode::Index => {
            let drive_infos = SyncIndex::invoke_preflight(drive_infos, if_exists)?;
            SyncIndex::invoke(drive_infos)
        }
        SyncMode::Both => {
            // The two stages have different skip/overwrite/abort filtering rules, so
            // they must each run their own preflight over the same initial drive set.
            let mft_drive_infos = SyncMft::invoke_preflight(drive_infos.clone(), if_exists)?;
            let index_drive_infos = SyncIndex::invoke_preflight(drive_infos, if_exists)?;

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

            let mft_span = info_span!("dispatch mft sync work");
            let mft_data = {
                let _guard = mft_span.enter();
                SyncMft::invoke(mft_drive_infos)?
            };

            let in_memory_index_drive_letters_for_stream =
                Arc::clone(&in_memory_index_drive_letters);
            let in_memory_indexing = async move {
                // Consume completed MFT reads as they arrive and fan index construction out
                // concurrently so slow drives do not block faster ones.
                tracing::debug!(
                    drive_count = in_memory_index_drive_letters_for_stream.len(),
                    "Collecting MFT sync and in-memory index results"
                );
                mft_data
                    .try_for_each_concurrent(None, move |(drive_info, physical_mft)| {
                        let in_memory_index_drive_letters =
                            Arc::clone(&in_memory_index_drive_letters_for_stream);
                        async move {
                            if !in_memory_index_drive_letters.contains(&drive_info.drive_letter) {
                                return Ok(());
                            }

                            let parent_span = Span::current();
                            tokio::task::spawn_blocking(move || -> eyre::Result<()> {
                                let _parent_guard = parent_span.enter();
                                let _guard = info_span!(
                                    "build_in_memory_search_index_for_drive",
                                    drive = %drive_info.drive_letter,
                                    index_path = %drive_info.index_output_path.display(),
                                )
                                .entered();
                                let mft_file = physical_mft.to_mft_file()?;
                                SyncIndex::invoke_for_mft_file(&drive_info, &mft_file)?;
                                {
                                    let _guard = info_span!(
                                        "drop_in_memory_index_inputs",
                                        drive = %drive_info.drive_letter,
                                        physical_segments = physical_mft.physical_read_results.entries.len(),
                                        logical_segments = physical_mft.logical_read_plan.segments.len(),
                                        mft_entries = mft_file.record_count(),
                                    )
                                    .entered();
                                    drop(mft_file);
                                    drop(physical_mft);
                                };
                                Ok(())
                            })
                            .await
                            .map_err(|error| {
                                eyre::eyre!("Failed joining in-memory index task: {error}")
                            })??;

                            Ok(())
                        }
                    })
                    .await
            }
            .instrument(mft_span);

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
                SyncIndex::invoke(fallback_index_drive_infos)
            };

            tokio::try_join!(in_memory_indexing, disk_indexing)?;

            Ok(())
        }
    }
}
