use crate::cli::command::sync::sync_cli::SyncArgs;
use crate::cli::command::sync::sync_common::resolve_drive_infos;
use crate::mft::mft_physical_read::read_physical_mft;
use eyre::Context;
use itertools::Itertools;
use teamy_windows::elevation::enable_backup_privileges;
use teamy_windows::elevation::ensure_elevated;
use tokio::task::JoinSet;
use tracing::info;
use tracing::info_span;

pub async fn invoke_sync_mft(args: &SyncArgs) -> eyre::Result<()> {
    ensure_elevated()?;
    enable_backup_privileges().wrap_err("Failed to enable backup privileges")?;

    let _span = info_span!("sync MFTs from disks to files").entered();

    let drive_infos = { resolve_drive_infos(&args.drive_letter_pattern, &args.if_exists)? };

    info!(
        "Found {} drives to sync: {}",
        drive_infos.len(),
        drive_infos.iter().map(|info| info.drive_letter).join(", ")
    );

    let mut join_set = JoinSet::new();

    for drive_info in drive_infos {
        join_set.spawn(async move {
            let mft = read_physical_mft(drive_info.drive_letter)?;
            mft.write_to_path(&drive_info.mft_output_path)?;
            eyre::Ok((drive_info, mft))
        });
    }

    while let Some(result) = join_set.join_next().await {
        let (drive_info, _read_results) = result??;
        info!("Finished syncing MFT for drive {}", drive_info.drive_letter);
    }

    info!("Finished syncing MFT files");
    Ok(())
}
