use crate::cli::command::sync::IfExistsOutputBehaviour;
use crate::cli::command::sync::drive_sync_info::DriveSyncInfo;
use crate::mft::mft_physical_read::PhysicalMftReadResult;
use crate::mft::mft_physical_read::read_physical_mft;
use arbitrary::Arbitrary;
use async_stream::try_stream;
use eyre::Context;
use eyre::bail;
use eyre::ensure;
use facet::Facet;
use itertools::Itertools;
use std::collections::BTreeMap;
use teamy_windows::elevation::enable_backup_privileges;
use teamy_windows::elevation::ensure_elevated;
use tokio_stream::Stream;
use tokio_stream::StreamExt;
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
        &self,
        drive_infos: BTreeMap<char, DriveSyncInfo>,
        if_exists: &IfExistsOutputBehaviour,
    ) -> eyre::Result<BTreeMap<char, DriveSyncInfo>> {
        let mut rtn = BTreeMap::default();
        for (drive_letter, info) in drive_infos {
            let mft_exists = info.mft_output_path.exists();
            match (mft_exists, if_exists) {
                (false, _) | (true, IfExistsOutputBehaviour::Overwrite) => {
                    let prev = rtn.insert(drive_letter, info);
                    ensure!(prev.is_none());
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
    pub async fn invoke(
        &self,
        drive_infos: BTreeMap<char, DriveSyncInfo>,
    ) -> eyre::Result<impl Stream<Item = eyre::Result<(DriveSyncInfo, PhysicalMftReadResult)>>>
    {
        ensure_elevated()?;
        enable_backup_privileges().wrap_err("Failed to enable backup privileges")?;

        info!(
            "Found {} drives to sync MFT files for: {}",
            drive_infos.len(),
            drive_infos
                .iter()
                .map(|(_, info)| info.drive_letter)
                .join(", ")
        );

        Ok(try_stream! {
            let _span = info_span!("sync MFTs from disks to files").entered();
            let physical_mft_stream = read_physical_mft_stream_with_info(drive_infos.into_values());
            tokio::pin!(physical_mft_stream);
            while let Some(mft) = physical_mft_stream.next().await {
                let (drive_info, mft_result) = mft?;
                mft_result.write_to_path(&drive_info.mft_output_path).wrap_err_with(|| {
                    format!(
                        "Failed writing MFT snapshot for drive {} to {}",
                        drive_info.drive_letter,
                        drive_info.mft_output_path.display()
                    )
                })?;
                yield (drive_info, mft_result);
            }
        })
    }
}

pub fn read_physical_mft_stream_with_info(
    drive_infos: impl IntoIterator<Item = DriveSyncInfo>,
) -> impl Stream<Item = eyre::Result<(DriveSyncInfo, PhysicalMftReadResult)>> {
    futures::stream::iter(drive_infos.into_iter()).then(|drive_info| async {
        let physical_mft_read_result =
            read_physical_mft(drive_info.drive_letter).wrap_err_with(|| {
                format!(
                    "Failed reading MFT data for drive {}",
                    drive_info.drive_letter
                )
            })?;
        Ok((drive_info, physical_mft_read_result))
    })
}
