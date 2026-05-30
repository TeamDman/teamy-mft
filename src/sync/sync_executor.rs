use crate::sync::DriveSyncInfo;
use crate::sync::IfExistsOutputBehaviour;
use crate::sync::SyncIndex;
use crate::sync::SyncMft;
use crate::sync::SyncMode;
use crate::sync::sync_phase::SyncPhase;
use crate::sync::sync_phase::bytes_human;
use crate::sync::sync_phase::bytes_per_second;
use crate::sync::sync_phase::bytes_per_second_human;
use crate::sync::sync_phase::count_per_second;
use crate::sync::sync_phase::count_per_second_human;
use crate::sync::sync_phase::elapsed_human;
use crate::sync::sync_phase::elapsed_ms;
use crate::sync::sync_phase::u64_from_usize;
use eyre::Context;
use std::collections::BTreeSet;
use tokio_stream::StreamExt;
use tracing::Instrument;
use tracing::Span;
use tracing::info;
use tracing::info_span;
use uom::si::information::byte;

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
                    let phase = SyncPhase::start(
                        "drop_physical_mft_read_result",
                        Some(drive_info.drive_letter),
                    );
                    let physical_segments = physical_mft.physical_read_results.entries.len();
                    let logical_segments = physical_mft.logical_read_plan.segments.len();
                    drop(physical_mft);
                    let elapsed = phase.elapsed();
                    info!(
                        phase = phase.name(),
                        drive = %phase.drive(),
                        elapsed_ms = elapsed_ms(elapsed),
                        elapsed_human = %elapsed_human(elapsed),
                        physical_segments,
                        logical_segments,
                        "Finished sync phase"
                    );
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
            let all_drive_infos = drive_infos.clone();
            let mft_drive_infos = SyncMft::invoke_preflight(drive_infos.clone(), if_exists)?;
            let index_drive_infos = SyncIndex::invoke_preflight(drive_infos, if_exists)?;
            SyncMft::prepare_access()?;

            let mft_drive_letters = mft_drive_infos
                .iter()
                .map(|info| info.drive_letter)
                .collect::<BTreeSet<_>>();
            let index_drive_letters = index_drive_infos
                .iter()
                .map(|info| info.drive_letter)
                .collect::<BTreeSet<_>>();
            let planned_drive_count = all_drive_infos
                .iter()
                .filter(|info| {
                    mft_drive_letters.contains(&info.drive_letter)
                        || index_drive_letters.contains(&info.drive_letter)
                })
                .count();
            let sync_phase = SyncPhase::start("sync_both", None);
            info!(
                phase = sync_phase.name(),
                drive = %sync_phase.drive(),
                planned_drive_count,
                mft_drive_count = mft_drive_letters.len(),
                index_drive_count = index_drive_letters.len(),
                "Prepared sync phase"
            );

            let mft_span = info_span!("dispatch mft sync work");
            let mut drive_index = 0usize;
            for drive_info in all_drive_infos {
                let needs_mft = mft_drive_letters.contains(&drive_info.drive_letter);
                let needs_index = index_drive_letters.contains(&drive_info.drive_letter);
                if !needs_mft && !needs_index {
                    continue;
                }

                drive_index += 1;
                let drive_letter = drive_info.drive_letter;
                let drive_phase = SyncPhase::start("sync_drive", Some(drive_letter));
                info!(
                    phase = drive_phase.name(),
                    drive = %drive_phase.drive(),
                    drive_index,
                    drive_count = planned_drive_count,
                    needs_mft,
                    needs_index,
                    "Prepared sync drive"
                );
                let parent_span = Span::current();
                let mft_span_for_drive = mft_span.clone();
                tokio::task::spawn_blocking(move || -> eyre::Result<()> {
                    let _parent_guard = parent_span.enter();
                    let _mft_guard = needs_mft.then(|| mft_span_for_drive.enter());

                    if needs_mft {
                        let (drive_info, physical_mft) = SyncMft::invoke_for_drive(drive_info)?;
                        if needs_index {
                            let materialize_phase =
                                SyncPhase::start("materialize_mft_for_index", Some(drive_info.drive_letter));
                            let mft_file = physical_mft.to_mft_file().wrap_err_with(|| {
                                format!(
                                    "Failed materializing MFT data for drive {} for index build",
                                    drive_info.drive_letter
                                )
                            })?;
                            let materialize_elapsed = materialize_phase.elapsed();
                            let source_mft_bytes = u64_from_usize(mft_file.size().get::<byte>());
                            let source_mft_entries = mft_file.record_count();
                            info!(
                                phase = materialize_phase.name(),
                                drive = %materialize_phase.drive(),
                                elapsed_ms = elapsed_ms(materialize_elapsed),
                                elapsed_human = %elapsed_human(materialize_elapsed),
                                source_mft_bytes,
                                source_mft_human = %bytes_human(source_mft_bytes),
                                source_mft_entries,
                                bytes_per_second = bytes_per_second(source_mft_bytes, materialize_elapsed),
                                bytes_per_second_human = %bytes_per_second_human(source_mft_bytes, materialize_elapsed),
                                entries_per_second = count_per_second(source_mft_entries, materialize_elapsed),
                                entries_per_second_human = %count_per_second_human(source_mft_entries, materialize_elapsed),
                                "Finished sync phase"
                            );

                            let physical_segments = physical_mft.physical_read_results.entries.len();
                            let logical_segments = physical_mft.logical_read_plan.segments.len();
                            let drop_phase = SyncPhase::start(
                                "drop_physical_mft_read_result",
                                Some(drive_info.drive_letter),
                            );
                            drop(physical_mft);
                            let drop_elapsed = drop_phase.elapsed();
                            info!(
                                phase = drop_phase.name(),
                                drive = %drop_phase.drive(),
                                elapsed_ms = elapsed_ms(drop_elapsed),
                                elapsed_human = %elapsed_human(drop_elapsed),
                                physical_segments,
                                logical_segments,
                                "Finished sync phase"
                            );

                            SyncIndex::invoke_for_mft_file(&drive_info, &mft_file)?;

                            let drop_phase =
                                SyncPhase::start("drop_mft_file_after_index", Some(drive_info.drive_letter));
                            drop(mft_file);
                            let drop_elapsed = drop_phase.elapsed();
                            info!(
                                phase = drop_phase.name(),
                                drive = %drop_phase.drive(),
                                elapsed_ms = elapsed_ms(drop_elapsed),
                                elapsed_human = %elapsed_human(drop_elapsed),
                                source_mft_bytes,
                                source_mft_entries,
                                "Finished sync phase"
                            );
                        } else {
                            let physical_segments = physical_mft.physical_read_results.entries.len();
                            let logical_segments = physical_mft.logical_read_plan.segments.len();
                            let drop_phase = SyncPhase::start(
                                "drop_physical_mft_read_result",
                                Some(drive_info.drive_letter),
                            );
                            drop(physical_mft);
                            let drop_elapsed = drop_phase.elapsed();
                            info!(
                                phase = drop_phase.name(),
                                drive = %drop_phase.drive(),
                                elapsed_ms = elapsed_ms(drop_elapsed),
                                elapsed_human = %elapsed_human(drop_elapsed),
                                physical_segments,
                                logical_segments,
                                "Finished sync phase"
                            );
                        }
                    } else if needs_index {
                        SyncIndex::invoke_for_mft_path(&drive_info)?;
                    }

                    Ok(())
                })
                .await
                .map_err(|error| {
                    eyre::eyre!("Failed joining sync task for drive {drive_letter}: {error}")
                })??;

                let drive_elapsed = drive_phase.elapsed();
                info!(
                    phase = drive_phase.name(),
                    drive = %drive_phase.drive(),
                    drive_index,
                    drive_count = planned_drive_count,
                    needs_mft,
                    needs_index,
                    elapsed_ms = elapsed_ms(drive_elapsed),
                    elapsed_human = %elapsed_human(drive_elapsed),
                    "Finished sync phase"
                );
            }

            let sync_elapsed = sync_phase.elapsed();
            info!(
                phase = sync_phase.name(),
                drive = %sync_phase.drive(),
                planned_drive_count,
                mft_drive_count = mft_drive_letters.len(),
                index_drive_count = index_drive_letters.len(),
                elapsed_ms = elapsed_ms(sync_elapsed),
                elapsed_human = %elapsed_human(sync_elapsed),
                "Finished sync phase"
            );

            Ok(())
        }
    }
}
