use crate::cli::command::sync::sync_args::SyncArgs;
use crate::cli::command::sync::sync_common::DriveSnapshot;
use crate::cli::command::sync::sync_common::resolve_drive_infos;
use crate::mft::mft_physical_read::read_physical_mft;
use crossbeam_channel::bounded;
use eyre::Context;
use itertools::Itertools;
use std::io::Cursor;
use std::path::PathBuf;
use std::thread;
use teamy_windows::elevation::enable_backup_privileges;
use teamy_windows::elevation::ensure_elevated;
use tracing::debug;
use tracing::error;
use tracing::info;

pub(crate) fn invoke_sync_mft(args: &SyncArgs, capture_bytes: bool) -> eyre::Result<Vec<DriveSnapshot>> {
    ensure_elevated()?;
    enable_backup_privileges().wrap_err("Failed to enable backup privileges")?;

    let drive_infos = resolve_drive_infos(&args.drive_pattern, &args.if_exists, true)?;

    info!(
        "Found {} drives to sync: {}",
        drive_infos.len(),
        drive_infos.iter().map(|info| info.drive_letter).join(", ")
    );

    let max_workers = drive_infos.len();
    let (tx, rx) = bounded::<(char, PathBuf)>(drive_infos.len());
    let (snapshot_tx, snapshot_rx) = bounded::<DriveSnapshot>(drive_infos.len());

    let mut handles = Vec::with_capacity(max_workers);
    for worker_id in 0..max_workers {
        let rx = rx.clone();
        let snapshot_tx = snapshot_tx.clone();
        let handle = thread::Builder::new()
            .name(format!("mft-iocp-{worker_id}"))
            .spawn(move || {
                while let Ok((drive_letter, output_path)) = rx.recv() {
                    info!(
                        "Worker {} reading drive {} -> {}",
                        worker_id,
                        drive_letter,
                        output_path.display()
                    );

                    match read_physical_mft(drive_letter) {
                        Ok((logical_segments, physical_read_results)) => {
                            if capture_bytes {
                                let mut cursor = Cursor::new(Vec::<u8>::new());
                                physical_read_results
                                    .read_into_writer(&logical_segments, &mut cursor)
                                    .wrap_err("Failed constructing in-memory MFT bytes")?;
                                let bytes = cursor.into_inner();

                                std::fs::write(&output_path, &bytes)
                                    .wrap_err("Failed writing MFT output file")?;

                                snapshot_tx
                                    .send(DriveSnapshot {
                                        drive_letter,
                                        bytes,
                                    })
                                    .wrap_err("Failed sending drive snapshot")?;
                            } else {
                                physical_read_results
                                    .read_into_path(&logical_segments, &output_path)
                                    .wrap_err("Failed writing MFT output file")?;
                            }
                        }
                        Err(e) => {
                            error!(
                                "Worker {}: IOCP read failed for {}: {:#}",
                                worker_id, drive_letter, e
                            );
                        }
                    }
                }
                eyre::Ok(())
            })
            .wrap_err("Failed to spawn IOCP worker thread")
            .unwrap();
        handles.push(handle);
    }
    drop(snapshot_tx);

    for info in &drive_infos {
        tx.send((info.drive_letter, info.output_path.clone()))
            .wrap_err("Failed to schedule IOCP drive job")?;
    }
    drop(tx);

    for handle in handles {
        handle
            .join()
            .map_err(|e| eyre::eyre!("Failed to join worker: {:?}", e))?
            .wrap_err("Identified failure result from worker")?;
    }

    let mut snapshots = Vec::new();
    if capture_bytes {
        while let Ok(snapshot) = snapshot_rx.recv() {
            snapshots.push(snapshot);
        }
        snapshots.sort_by_key(|s| s.drive_letter);
        debug!(
            drives_with_snapshots = snapshots.len(),
            "Collected in-memory MFT snapshots"
        );
    }

    info!("MFT sync stage completed");

    Ok(snapshots)
}
