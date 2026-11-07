use crate::mft::PhysicalMftReadRequest;
use crate::read::CancellableSeekWriter;
use crate::sync::DriveSyncInfo;
use crate::sync::IfExistsOutputBehaviour;
use crate::windows_utils::elevation::enable_backup_privileges;
use crate::windows_utils::elevation::ensure_elevated;
use eyre::Context;
use eyre::bail;
use futures::StreamExt as _;
use futures::stream;
use itertools::Itertools;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use teamy_cancellation::CancellationScope;
use tokio_stream::Stream;
use tracing::debug;
use tracing::info;
use tracing::info_span;

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
        console_pid: Option<u32>,
        cancel: teamy_cancellation::CancellationScope,
    ) -> eyre::Result<impl Stream<Item = eyre::Result<DriveSyncInfo>>> {
        if !drive_infos.is_empty() {
            ensure_elevated(console_pid)?;
            enable_backup_privileges().wrap_err("Failed to enable backup privileges")?;
        }
        info!(
            "Found {} drives to sync MFT files for: {}",
            drive_infos.len(),
            drive_infos.iter().map(|info| info.drive_letter).join(", ")
        );
        Ok(read_physical_mft_stream_with_info(drive_infos, cancel))
    }

    pub(crate) fn publish_snapshot(
        output_path: &Path,
        cancel: &CancellationScope,
        produce: impl FnOnce(&mut CancellableSeekWriter<'_, File>) -> eyre::Result<()>,
        flush: impl FnOnce(&File) -> eyre::Result<()>,
    ) -> eyre::Result<()> {
        cancel.bail_if_cancelled()?;
        let parent = output_path.parent().unwrap_or_else(|| Path::new("."));
        let mut snapshot = crate::machine::security::create_private_snapshot(parent)?;
        {
            let mut writer = CancellableSeekWriter::new(snapshot.as_file_mut(), cancel);
            produce(&mut writer)?;
            writer.flush()?;
        }
        cancel.bail_if_cancelled()?;
        flush(snapshot.as_file())?;
        cancel.bail_if_cancelled()?;
        // Keep staging private through acquisition and flush, then apply the final policy.
        if output_path.try_exists()? {
            crate::machine::security::copy_path_dacl(output_path, snapshot.path())?;
        } else {
            crate::machine::security::inherit_new_file_dacl(parent, snapshot.path())?;
        }
        cancel.bail_if_cancelled()?;
        snapshot.persist(output_path).map_err(|error| error.error)?;
        Ok(())
    }
}

pub fn read_physical_mft_stream_with_info(
    drive_infos: impl IntoIterator<Item = DriveSyncInfo>,
    cancel: teamy_cancellation::CancellationScope,
) -> impl Stream<Item = eyre::Result<DriveSyncInfo>> {
    let drive_infos = drive_infos.into_iter().collect::<Vec<_>>();
    let concurrency = drive_infos.len().max(1);
    stream::iter(drive_infos).map(move |drive_info| {
        let cancel = cancel.clone();
        async move {
            let parent = tracing::Span::current();
            tokio::task::spawn_blocking(move || -> eyre::Result<DriveSyncInfo> {
                let _parent = parent.enter();
                let _span = info_span!("stream_physical_mft_for_drive", drive = %drive_info.drive_letter).entered();
                SyncMft::publish_snapshot(
                    &drive_info.mft_output_path,
                    &cancel,
                    |writer| PhysicalMftReadRequest { drive_letter: drive_info.drive_letter }.execute(writer),
                    |file| Ok(file.sync_all()?),
                )
                    .wrap_err_with(|| format!("Failed streaming MFT for drive {}", drive_info.drive_letter))?;
                Ok(drive_info)
            }).await.map_err(|error| eyre::eyre!("Failed joining MFT read task: {error}"))?
        }
    }).buffer_unordered(concurrency)
}

#[cfg(test)]
mod tests {
    use super::SyncMft;
    use crate::cancellation::CancellationToken;
    use crate::mft::PhysicalMftReadRequest;
    use crate::read::byte_range::ByteRange;
    use crate::read::logical_read_plan::LogicalFileSegment;
    use crate::read::logical_read_plan::LogicalFileSegmentKind;
    use crate::read::logical_read_plan::LogicalReadPlan;
    use crate::read::physical_read_plan::PhysicalReadPlan;
    use crate::windows_utils::string::EasyPCWSTR;
    use std::io::Write;
    use std::os::windows::fs::OpenOptionsExt;
    use uom::si::information::byte;
    use uom::si::usize::Information;

    #[test]
    fn pre_cancelled_publication_does_not_create_staging_or_invoke_producer() -> eyre::Result<()> {
        let temp = tempfile::tempdir()?;
        let output = temp.path().join("snapshot.mft");
        std::fs::write(&output, b"previous snapshot")?;
        let token = CancellationToken::new();
        let mut produced = false;
        let result = token.with_scope(|scope| {
            token.request_cancel("before publication");
            SyncMft::publish_snapshot(
                &output,
                &scope,
                |_| {
                    produced = true;
                    Ok(())
                },
                |_| panic!("cancelled publication must not flush"),
            )
        });

        assert!(result.is_err());
        assert!(!produced);
        assert_eq!(std::fs::read(&output)?, b"previous snapshot");
        assert_eq!(std::fs::read_dir(temp.path())?.count(), 1);
        Ok(())
    }

    #[test]
    fn short_read_preserves_previous_snapshot_and_cleans_staging() -> eyre::Result<()> {
        let temp = tempfile::tempdir()?;
        let source = temp.path().join("source.bin");
        std::fs::write(&source, [0; 512])?;
        let source = source.easy_pcwstr()?;
        let output = temp.path().join("snapshot.mft");
        std::fs::write(&output, b"previous snapshot")?;
        let logical = LogicalReadPlan {
            segments: [LogicalFileSegment {
                logical_range: ByteRange::from_offset_and_length(
                    Information::new::<byte>(0),
                    Information::new::<byte>(1024),
                )?,
                kind: LogicalFileSegmentKind::Physical {
                    physical_offset: Information::new::<byte>(0),
                },
            }]
            .into_iter()
            .collect(),
        };
        let result = CancellationToken::new().with_scope(|scope| {
            SyncMft::publish_snapshot(
                &output,
                &scope,
                |writer| {
                    PhysicalMftReadRequest::write_plan(
                        PhysicalReadPlan::try_from(&logical)?,
                        &source,
                        writer,
                    )
                },
                |file| Ok(file.sync_all()?),
            )
        });

        assert!(result.is_err());
        assert_eq!(std::fs::read(&output)?, b"previous snapshot");
        assert_eq!(std::fs::read_dir(temp.path())?.count(), 2);
        Ok(())
    }

    #[test]
    fn cancellation_during_production_preserves_previous_snapshot() -> eyre::Result<()> {
        let temp = tempfile::tempdir()?;
        let output = temp.path().join("snapshot.mft");
        std::fs::write(&output, b"previous snapshot")?;
        let token = CancellationToken::new();
        let mut partial = false;
        let result = token.with_scope(|scope| {
            SyncMft::publish_snapshot(
                &output,
                &scope,
                |writer| {
                    writer.write_all(b"first chunk")?;
                    partial = true;
                    token.request_cancel("after first chunk");
                    writer.write_all(b"second chunk")?;
                    Ok(())
                },
                |_| panic!("cancelled production must not flush"),
            )
        });

        assert!(result.is_err());
        assert!(partial);
        assert_eq!(std::fs::read(&output)?, b"previous snapshot");
        assert_eq!(std::fs::read_dir(temp.path())?.count(), 1);
        Ok(())
    }

    #[test]
    fn staging_is_private_and_cancellation_during_flush_does_not_publish() -> eyre::Result<()> {
        let temp = tempfile::tempdir()?;
        let output = temp.path().join("snapshot.mft");
        std::fs::write(&output, b"previous snapshot")?;
        let token = CancellationToken::new();
        let sid = crate::machine::security::current_user_sid_string()?;
        let mut observed = false;
        let result = token.with_scope(|scope| {
            SyncMft::publish_snapshot(
                &output,
                &scope,
                |writer| {
                    let entries = std::fs::read_dir(temp.path())?
                        .map(|entry| entry.map(|entry| entry.path()))
                        .collect::<std::io::Result<Vec<_>>>()?;
                    let staging = entries.iter().find(|path| **path != output).unwrap();
                    let status =
                        crate::machine::security::query_path_protection_status(staging, &sid)?;
                    assert!(status.protection_enabled());
                    assert_eq!(std::fs::metadata(staging)?.len(), 0);
                    observed = true;
                    writer.write_all(b"replacement")?;
                    Ok(())
                },
                |file| {
                    token.request_cancel("during flush");
                    file.sync_all()?;
                    Ok(())
                },
            )
        });

        assert!(result.is_err());
        assert!(observed);
        assert_eq!(std::fs::read(&output)?, b"previous snapshot");
        assert_eq!(std::fs::read_dir(temp.path())?.count(), 1);
        Ok(())
    }

    #[test]
    fn failed_flush_preserves_previous_snapshot() -> eyre::Result<()> {
        let temp = tempfile::tempdir()?;
        let output = temp.path().join("snapshot.mft");
        std::fs::write(&output, b"previous snapshot")?;
        let result = CancellationToken::new().with_scope(|scope| {
            SyncMft::publish_snapshot(
                &output,
                &scope,
                |writer| Ok(writer.write_all(b"replacement")?),
                |_| eyre::bail!("injected flush failure"),
            )
        });

        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("injected flush failure")
        );
        assert_eq!(std::fs::read(&output)?, b"previous snapshot");
        assert_eq!(std::fs::read_dir(temp.path())?.count(), 1);
        Ok(())
    }

    #[test]
    fn publication_failure_preserves_previous_snapshot() -> eyre::Result<()> {
        let temp = tempfile::tempdir()?;
        let output = temp.path().join("snapshot.mft");
        std::fs::write(&output, b"previous snapshot")?;
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(windows::Win32::Storage::FileSystem::FILE_SHARE_READ.0)
            .open(&output)?;
        let result = CancellationToken::new().with_scope(|scope| {
            SyncMft::publish_snapshot(
                &output,
                &scope,
                |writer| Ok(writer.write_all(b"replacement")?),
                |file| Ok(file.sync_all()?),
            )
        });

        assert!(result.is_err());
        drop(held);
        assert_eq!(std::fs::read(&output)?, b"previous snapshot");
        assert_eq!(std::fs::read_dir(temp.path())?.count(), 1);
        Ok(())
    }

    #[test]
    fn successful_publication_handles_new_and_existing_snapshots() -> eyre::Result<()> {
        let temp = tempfile::tempdir()?;
        let output = temp.path().join("snapshot.mft");
        for replacement in [b"first snapshot".as_slice(), b"new"] {
            let mut flushed = false;
            CancellationToken::new().with_scope(|scope| {
                SyncMft::publish_snapshot(
                    &output,
                    &scope,
                    |writer| Ok(writer.write_all(replacement)?),
                    |file| {
                        assert_eq!(file.metadata()?.len(), replacement.len() as u64);
                        file.sync_all()?;
                        flushed = true;
                        Ok(())
                    },
                )
            })?;

            assert!(flushed);
            assert_eq!(std::fs::read(&output)?, replacement);
            assert_eq!(std::fs::read_dir(temp.path())?.count(), 1);
        }
        Ok(())
    }
}
