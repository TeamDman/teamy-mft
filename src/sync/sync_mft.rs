use crate::machine::DriveArtifactInfo;
use crate::mft::PhysicalMftReadRequest;
use crate::read::CancellableSeekWriter;
use crate::windows_utils::elevation::enable_backup_privileges;
use crate::windows_utils::elevation::ensure_elevated;
use eyre::Context;
use futures::StreamExt as _;
use futures::stream;
use itertools::Itertools;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use teamy_cancellation::CancellationScope;
use tokio_stream::Stream;
use tracing::info;
use tracing::info_span;

#[derive(Debug)]
pub struct SyncMft;

impl SyncMft {
    /// Sync MFT data from drives.
    ///
    /// The caller selects artifact records after preflight.
    ///
    /// # Errors
    ///
    /// Returns an error if elevation fails or if reading/writing MFT data fails.
    pub fn invoke<'a>(
        drive_infos: impl Iterator<Item = &'a DriveArtifactInfo> + Clone + 'a,
        console_pid: Option<u32>,
        cancel: CancellationScope,
    ) -> eyre::Result<impl Stream<Item = eyre::Result<DriveArtifactInfo>> + 'a> {
        let drive_count = drive_infos.clone().count();
        if drive_count > 0 {
            ensure_elevated(console_pid)?;
            enable_backup_privileges().wrap_err("Failed to enable backup privileges")?;
        }
        info!(
            "Found {} drives to sync MFT files for: {}",
            drive_count,
            drive_infos.clone().map(|info| info.drive_letter).join(", ")
        );
        Ok(stream::iter(drive_infos).map(move |drive_info| {
            let cancel = cancel.clone();
            async move {
                let drive_info = drive_info.clone();
                let parent = tracing::Span::current();
                tokio::task::spawn_blocking(move || -> eyre::Result<DriveArtifactInfo> {
                    let _parent = parent.enter();
                    let _span = info_span!("stream_physical_mft_for_drive", drive = %drive_info.drive_letter).entered();
                    Self::publish_snapshot(
                        &drive_info.mft_path,
                        &cancel,
                        |writer| PhysicalMftReadRequest { drive_letter: drive_info.drive_letter }.execute(writer),
                        |file| Ok(file.sync_all()?),
                    )
                        .wrap_err_with(|| format!("Failed streaming MFT for drive {}", drive_info.drive_letter))?;
                    Ok(drive_info)
                }).await.map_err(|error| eyre::eyre!("Failed joining MFT read task: {error}"))?
            }
        }).buffer_unordered(drive_count.max(1)))
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
            writer.flush()
        }?;
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

        let _error =
            result.expect_err("pre-cancelled publication must fail before producing a snapshot");
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

        let _error = result.expect_err("a short physical read must prevent snapshot publication");
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

        let _error =
            result.expect_err("cancellation during production must prevent snapshot publication");
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

        let _error =
            result.expect_err("cancellation during flush must prevent snapshot publication");
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

        let _error = result.expect_err("the held destination must prevent snapshot replacement");
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
