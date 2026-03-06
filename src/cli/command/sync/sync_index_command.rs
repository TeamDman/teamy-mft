use crate::cli::command::sync::sync_args::IfExistsOutputBehaviour;
use crate::cli::command::sync::sync_args::SyncArgs;
use crate::cli::command::sync::sync_common::DriveSnapshot;
use crate::cli::command::sync::sync_common::resolve_drive_infos;
use crate::mft_process::process_mft_file;
use crate::search_index::format::SearchIndexHeader;
use crate::search_index::format::SearchIndexPathRow;
use eyre::Context;
use eyre::bail;
use itertools::Itertools;
use std::collections::HashMap;
use tracing::debug;
use tracing::info;

pub(crate) fn invoke_sync_index(
    args: &SyncArgs,
    snapshots: Option<&[DriveSnapshot]>,
) -> eyre::Result<()> {
    let drive_infos = resolve_drive_infos(&args.drive_pattern, &args.if_exists)?;
    let snapshot_map: HashMap<char, &Vec<u8>> = snapshots
        .unwrap_or(&[])
        .iter()
        .map(|snapshot| (snapshot.drive_letter, &snapshot.bytes))
        .collect();

    info!(
        "Building search indexes for drives: {}",
        drive_infos.iter().map(|info| info.drive_letter).join(", ")
    );

    for info in drive_infos {
        let index_path = info.index_output_path;
        match (index_path.exists(), &args.if_exists) {
            (true, IfExistsOutputBehaviour::Skip) => {
                debug!(
                    drive = %info.drive_letter,
                    path = %index_path.display(),
                    "Skipping existing index output"
                );
                continue;
            }
            (false, _) | (true, IfExistsOutputBehaviour::Overwrite) => {}
            (true, IfExistsOutputBehaviour::Abort) => {
                bail!("Aborting sync: {} already exists", index_path.display())
            }
        }

        let mft_bytes = if let Some(bytes) = snapshot_map.get(&info.drive_letter) {
            (*bytes).clone()
        } else {
            if !info.mft_output_path.is_file() {
                bail!(
                    "Cannot build index for drive {}: missing {}",
                    info.drive_letter,
                    info.mft_output_path.display()
                );
            }
            std::fs::read(&info.mft_output_path).wrap_err_with(|| {
                format!(
                    "Failed reading MFT snapshot for drive {} from {}",
                    info.drive_letter,
                    info.mft_output_path.display()
                )
            })?
        };

        let drive_name = info.drive_letter.to_string();
        let files = process_mft_file(&drive_name, &info.mft_output_path).wrap_err_with(|| {
            format!(
                "Failed processing MFT data for drive {} from {}",
                info.drive_letter,
                info.mft_output_path.display()
            )
        })?;

        let rows: Vec<SearchIndexPathRow> = files
            .0
            .into_iter()
            .flatten()
            .map(|path| SearchIndexPathRow {
                path: path.path.to_string_lossy().into_owned(),
                has_deleted_entries: path.has_deleted_entries(),
            })
            .collect();

        SearchIndexHeader::new(info.drive_letter, mft_bytes.len() as u64, rows.len() as u64)
            .write_to_path(&index_path, &rows)
            .wrap_err_with(|| {
                format!(
                    "Failed writing index output for drive {} to {}",
                    info.drive_letter,
                    index_path.display()
                )
            })?;
    }

    info!("Index sync stage completed");

    Ok(())
}
