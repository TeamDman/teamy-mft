use crate::cli::command::sync::IfExistsOutputBehaviour;
use crate::cli::command::sync::drive_sync_info::DriveSyncInfo;
use crate::mft::mft_physical_read::PhysicalMftReadResult;
use crate::mft::mft_physical_read::read_physical_mft;
use arbitrary::Arbitrary;
use async_stream::try_stream;
use eyre::Context;
use eyre::bail;
use facet::Facet;
use futures::StreamExt as _;
use futures::stream;
use itertools::Itertools;
use teamy_windows::elevation::enable_backup_privileges;
use teamy_windows::elevation::ensure_elevated;
use tokio_stream::Stream;
use tracing::debug;
use tracing::info;
use tracing::info_span;

#[derive(Facet, PartialEq, Debug, Arbitrary, Default, Clone)]
pub struct SyncMftArgs;

impl SyncMftArgs {
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
        ensure_elevated()?;
        enable_backup_privileges().wrap_err("Failed to enable backup privileges")?;

        info!(
            "Found {} drives to sync MFT files for: {}",
            drive_infos.len(),
            drive_infos.iter().map(|info| info.drive_letter).join(", ")
        );

        Ok(try_stream! {
            let _span = info_span!("sync MFTs from disks to files").entered();
            let physical_mft_stream = read_physical_mft_stream_with_info(drive_infos);
            tokio::pin!(physical_mft_stream);
            while let Some(mft) = physical_mft_stream.next().await {
                let (drive_info, mft_result) = mft?;
                {
                    let _span = info_span!(
                        "write_mft_snapshot_for_drive",
                        drive = %drive_info.drive_letter,
                        output_path = %drive_info.mft_output_path.display(),
                    )
                    .entered();
                    mft_result.write_to_path(&drive_info.mft_output_path).wrap_err_with(|| {
                        format!(
                            "Failed writing MFT snapshot for drive {} to {}",
                            drive_info.drive_letter,
                            drive_info.mft_output_path.display()
                        )
                    })?;
                }
                yield (drive_info, mft_result);
            }
        })
    }
}

pub fn read_physical_mft_stream_with_info(
    drive_infos: impl IntoIterator<Item = DriveSyncInfo>,
) -> impl Stream<Item = eyre::Result<(DriveSyncInfo, PhysicalMftReadResult)>> {
    let drive_infos = drive_infos.into_iter().collect::<Vec<_>>();
    let concurrency = drive_infos.len().max(1);

    stream::iter(drive_infos)
        .map(|drive_info| async move {
            tokio::task::spawn_blocking(
                move || -> eyre::Result<(DriveSyncInfo, PhysicalMftReadResult)> {
                    let _span = info_span!(
                        "read_physical_mft_for_drive",
                        drive = %drive_info.drive_letter,
                    )
                    .entered();
                    let physical_mft_read_result = read_physical_mft(drive_info.drive_letter)
                        .wrap_err_with(|| {
                            format!(
                                "Failed reading MFT data for drive {}",
                                drive_info.drive_letter
                            )
                        })?;
                    eyre::Ok((drive_info, physical_mft_read_result))
                },
            )
            .await
            .map_err(|error| eyre::eyre!("Failed joining MFT read task: {error}"))?
        })
        .buffer_unordered(concurrency)
}
