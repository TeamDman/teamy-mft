use crate::mft::mft_physical_read::PhysicalMftReadResult;
use crate::mft::mft_physical_read::read_physical_mft;
use crate::sync::DriveSyncInfo;
use crate::sync::IfExistsOutputBehaviour;
use crate::sync::sync_phase::SyncPhase;
use crate::sync::sync_phase::bytes_human;
use crate::sync::sync_phase::bytes_per_second;
use crate::sync::sync_phase::bytes_per_second_human;
use crate::sync::sync_phase::elapsed_human;
use crate::sync::sync_phase::elapsed_ms;
use crate::sync::sync_phase::u64_from_usize;
use crate::windows_utils::elevation::enable_backup_privileges;
use crate::windows_utils::elevation::ensure_elevated;
use async_stream::try_stream;
use eyre::Context;
use eyre::bail;
use futures::StreamExt as _;
use futures::stream;
use itertools::Itertools;
use tokio_stream::Stream;
use tracing::debug;
use tracing::info;
use tracing::info_span;
use uom::si::information::byte;

#[derive(Debug)]
pub struct SyncMft;

impl SyncMft {
    /// Validate the sync can proceed before any MFT reads begin.
    ///
    /// # Errors
    ///
    /// Returns an error if `if_exists` is `Abort` and any MFT output already exists.
    pub fn invoke_preflight(
        drive_infos: Vec<DriveSyncInfo>,
        if_exists: &IfExistsOutputBehaviour,
    ) -> eyre::Result<Vec<DriveSyncInfo>> {
        let mut rtn = Vec::with_capacity(drive_infos.len());
        for info in drive_infos {
            let mft_exists = info.mft_output_path.exists();
            match (mft_exists, if_exists) {
                (false, _) | (true, IfExistsOutputBehaviour::Overwrite) => {
                    rtn.push(info);
                }
                (true, IfExistsOutputBehaviour::Skip) => {
                    debug!(
                        drive = %info.drive_letter,
                        path = %info.mft_output_path.display(),
                        "Skipping existing MFT output"
                    );
                }
                (true, IfExistsOutputBehaviour::Abort) => {
                    bail!(
                        "Aborting sync: {} already exists",
                        info.mft_output_path.display()
                    );
                }
            }
        }
        Ok(rtn)
    }

    /// Sync MFT data from drives.
    ///
    /// Does not call the preflight check.
    ///
    /// # Errors
    ///
    /// Returns an error if the sync directory cannot be retrieved, elevation fails,
    /// or if reading/writing MFT data fails.
    pub fn invoke(
        drive_infos: Vec<DriveSyncInfo>,
    ) -> eyre::Result<impl Stream<Item = eyre::Result<(DriveSyncInfo, PhysicalMftReadResult)>>>
    {
        Self::prepare_access()?;

        info!(
            "Found {} drives to sync MFT files for: {}",
            drive_infos.len(),
            drive_infos.iter().map(|info| info.drive_letter).join(", ")
        );

        let sync_phase = SyncPhase::start("sync_mft", None);
        let drive_count = drive_infos.len();

        Ok(try_stream! {
            tracing::debug!("Syncing MFTs from disks to files");
            let physical_mft_stream = read_physical_mft_stream_with_info(drive_infos);
            tokio::pin!(physical_mft_stream);
            while let Some(mft) = physical_mft_stream.next().await {
                let (drive_info, mft_result) = mft?;
                Self::write_physical_mft_for_drive(&drive_info, &mft_result)?;
                yield (drive_info, mft_result);
            }
            let elapsed = sync_phase.elapsed();
            info!(
                phase = sync_phase.name(),
                drive = %sync_phase.drive(),
                drive_count,
                elapsed_ms = elapsed_ms(elapsed),
                elapsed_human = %elapsed_human(elapsed),
                "Finished sync phase"
            );
        })
    }

    /// Ensure the current process has the privileges needed for raw MFT reads.
    ///
    /// # Errors
    ///
    /// Returns an error if the process is not elevated or backup privileges cannot be enabled.
    pub(crate) fn prepare_access() -> eyre::Result<()> {
        ensure_elevated()?;
        enable_backup_privileges().wrap_err("Failed to enable backup privileges")?;
        Ok(())
    }

    /// Read and write the MFT snapshot for one drive.
    ///
    /// # Errors
    ///
    /// Returns an error if the raw drive cannot be read or the snapshot cannot be written.
    pub(crate) fn invoke_for_drive(
        drive_info: DriveSyncInfo,
    ) -> eyre::Result<(DriveSyncInfo, PhysicalMftReadResult)> {
        let physical_mft_read_result = Self::read_physical_mft_for_drive(drive_info.drive_letter)?;
        Self::write_physical_mft_for_drive(&drive_info, &physical_mft_read_result)?;
        Ok((drive_info, physical_mft_read_result))
    }

    fn read_physical_mft_for_drive(drive_letter: char) -> eyre::Result<PhysicalMftReadResult> {
        let _span = info_span!(
            "read_physical_mft_for_drive",
            drive = %drive_letter,
        )
        .entered();
        let phase = SyncPhase::start("read_physical_mft", Some(drive_letter));
        let physical_mft_read_result = read_physical_mft(drive_letter)
            .wrap_err_with(|| format!("Failed reading MFT data for drive {drive_letter}"))?;
        let elapsed = phase.elapsed();
        let logical_size_bytes = u64_from_usize(
            physical_mft_read_result
                .logical_read_plan
                .total_logical_size()
                .get::<byte>(),
        );
        let physical_size_bytes = u64_from_usize(
            physical_mft_read_result
                .physical_read_results
                .entries
                .iter()
                .map(|entry| entry.request.length.get::<byte>())
                .sum(),
        );
        info!(
            phase = phase.name(),
            drive = %phase.drive(),
            elapsed_ms = elapsed_ms(elapsed),
            elapsed_human = %elapsed_human(elapsed),
            logical_size_bytes,
            logical_size_human = %bytes_human(logical_size_bytes),
            physical_size_bytes,
            physical_size_human = %bytes_human(physical_size_bytes),
            bytes_per_second = bytes_per_second(physical_size_bytes, elapsed),
            bytes_per_second_human = %bytes_per_second_human(physical_size_bytes, elapsed),
            logical_segments = physical_mft_read_result.logical_read_plan.segments.len(),
            physical_segments = physical_mft_read_result.physical_read_results.entries.len(),
            "Finished sync phase"
        );
        Ok(physical_mft_read_result)
    }

    fn write_physical_mft_for_drive(
        drive_info: &DriveSyncInfo,
        mft_result: &PhysicalMftReadResult,
    ) -> eyre::Result<()> {
        debug!(
            drive = %drive_info.drive_letter,
            output_path = %drive_info.mft_output_path.display(),
            "Writing MFT snapshot for drive"
        );
        let phase = SyncPhase::start("write_physical_mft", Some(drive_info.drive_letter));
        mft_result
            .write_to_path(&drive_info.mft_output_path)
            .wrap_err_with(|| {
                format!(
                    "Failed writing MFT snapshot for drive {} to {}",
                    drive_info.drive_letter,
                    drive_info.mft_output_path.display()
                )
            })?;
        let elapsed = phase.elapsed();
        let logical_size_bytes = u64_from_usize(
            mft_result
                .logical_read_plan
                .total_logical_size()
                .get::<byte>(),
        );
        info!(
            phase = phase.name(),
            drive = %phase.drive(),
            elapsed_ms = elapsed_ms(elapsed),
            elapsed_human = %elapsed_human(elapsed),
            logical_size_bytes,
            logical_size_human = %bytes_human(logical_size_bytes),
            bytes_per_second = bytes_per_second(logical_size_bytes, elapsed),
            bytes_per_second_human = %bytes_per_second_human(logical_size_bytes, elapsed),
            logical_segments = mft_result.logical_read_plan.segments.len(),
            physical_segments = mft_result.physical_read_results.entries.len(),
            output_path = %drive_info.mft_output_path.display(),
            "Finished sync phase"
        );
        Ok(())
    }
}

pub fn read_physical_mft_stream_with_info(
    drive_infos: impl IntoIterator<Item = DriveSyncInfo>,
) -> impl Stream<Item = eyre::Result<(DriveSyncInfo, PhysicalMftReadResult)>> {
    let drive_infos = drive_infos.into_iter().collect::<Vec<_>>();

    stream::iter(drive_infos)
        .map(|drive_info| async move {
            let parent_span = tracing::Span::current();
            tokio::task::spawn_blocking(
                move || -> eyre::Result<(DriveSyncInfo, PhysicalMftReadResult)> {
                    let _parent_guard = parent_span.enter();
                    let physical_mft_read_result =
                        SyncMft::read_physical_mft_for_drive(drive_info.drive_letter)?;
                    Ok((drive_info, physical_mft_read_result))
                },
            )
            .await
            .map_err(|error| eyre::eyre!("Failed joining MFT read task: {error}"))?
        })
        .buffered(1)
}
