use crate::domain::DriveLetter;
use crate::mft::mft_record::MftRecord;
use crate::mft::mft_record_attribute_run_list::MftRecordAttributeRunListOwned;
use crate::mft::mft_record_number::MftRecordNumber;
use crate::mft::mft_record_size::MftRecordSize;
use crate::ntfs::ntfs_boot_sector::NtfsBootSector;
use crate::ntfs::ntfs_drive_handle::NtfsDriveHandle;
use crate::read::logical_read_plan::LogicalReadPlan;
use crate::read::physical_read_plan::PhysicalReadPlan;
use crate::read::physical_reader::PhysicalReader;
use crate::windows_utils::handle::get_read_only_drive_handle;
use crate::windows_utils::string::EasyPCWSTR;
use eyre::WrapErr;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use tracing::info;
use tracing::info_span;
use tracing::instrument;
use uom::si::information::byte;
use uom::si::information::mebibyte;
use uom::si::usize::Information;

/// A request to read a drive's physical `$MFT` stream.
#[derive(Debug)]
pub struct PhysicalMftReadRequest {
    pub drive_letter: DriveLetter,
}

impl PhysicalMftReadRequest {
    /// Stream the logical MFT into a seekable writer using bounded IOCP reads.
    ///
    /// Writes use absolute logical offsets. The caller owns destination creation,
    /// truncation, cancellation binding, flushing and publication. Sparse ranges
    /// and gaps are explicitly zero-filled.
    ///
    /// # Errors
    ///
    /// Returns planning, read, seek or writer errors. Failure can leave partial output.
    #[instrument(skip_all, fields(drive_letter = %self.drive_letter))]
    pub fn execute(self, writer: &mut (impl Write + Seek)) -> eyre::Result<()> {
        let volume_path = format!(r"\\.\{}:", self.drive_letter).easy_pcwstr()?;
        let logical = self.prepare_logical_plan()?;
        let chunk_size = Information::new::<mebibyte>(1);
        let physical = {
            let _span = info_span!(
                "build_physical_mft_read_plan",
                drive = %self.drive_letter,
                logical_segments = logical.segments.len(),
                chunk_size_bytes = chunk_size.get::<byte>(),
            )
            .entered();
            let mut physical = logical.as_physical_read_plan();
            physical.align_512();
            physical.chunked(chunk_size)
        };
        let count = physical.len();
        let size = logical.total_logical_size().get::<byte>();
        Self::write_plan(physical, &volume_path, writer)?;
        info!(drive = %self.drive_letter, physical_requests = count, logical_bytes = size, "Streamed MFT to writer");
        Ok(())
    }

    fn prepare_logical_plan(&self) -> eyre::Result<LogicalReadPlan> {
        let drive_letter = self.drive_letter;
        // Open blocking handle for boot sector & MFT record parsing
        let drive_handle: NtfsDriveHandle = {
            let _span = info_span!("open_ntfs_drive_handle", drive = %drive_letter).entered();
            get_read_only_drive_handle(drive_letter)
                .wrap_err("Failed to open handle to drive")?
                .try_into()
                .wrap_err("Failed to convert drive handle to NtfsDriveHandle")?
        };

        let boot_sector = {
            let _span = info_span!("read_ntfs_boot_sector", drive = %drive_letter).entered();
            NtfsBootSector::try_from_handle(&drive_handle)?
        };
        let mft_record_size = MftRecordSize::new(boot_sector.file_record_size())?;
        let dollar_mft_record = {
            let _span = info_span!(
                "read_dollar_mft_record",
                drive = %drive_letter,
                record_size_bytes = mft_record_size.get::<byte>(),
            )
            .entered();
            MftRecord::try_from_handle(
                &drive_handle,
                boot_sector
                    .mft_location()
                    .record_location(MftRecordNumber::DOLLAR_MFT, boot_sector.file_record_size()),
                mft_record_size,
            )
            .wrap_err("Failed reading record")?
        };

        // Gather all non-resident $DATA runlists (could be multiple segments if attribute list used).
        let decoded_runs = {
            let _span = info_span!("decode_dollar_mft_runlists", drive = %drive_letter).entered();
            MftRecordAttributeRunListOwned::from_mft_record(&dollar_mft_record)?
        };
        if decoded_runs.is_empty() {
            eyre::bail!("No non-resident $DATA runs found in $MFT record");
        }
        drop(drive_handle);

        // Build sparse-aware logical plan
        let logical_read_plan = {
            let _span = info_span!(
                "build_logical_mft_read_plan",
                drive = %drive_letter,
                bytes_per_cluster = boot_sector.bytes_per_cluster(),
            )
            .entered();
            decoded_runs
                .into_logical_read_plan(Information::new::<byte>(boot_sector.bytes_per_cluster()))
        };
        if logical_read_plan.segments.is_empty() {
            eyre::bail!("Logical plan empty (no runs)");
        }

        Ok(logical_read_plan)
    }

    pub(crate) fn write_plan(
        physical: PhysicalReadPlan<'_>,
        source: impl windows::core::Param<windows::core::PCWSTR>,
        writer: &mut (impl Write + Seek),
    ) -> eyre::Result<()> {
        let max_in_flight = PhysicalReader::configured_max_in_flight_io()?;
        let logical = physical.logical_plan();
        let size = logical.total_logical_size().get::<byte>();
        let _span = info_span!(
            "execute_physical_mft_read_plan",
            physical_requests = physical.len(),
            total_physical_bytes = physical.total_size().get::<byte>(),
            logical_bytes = size,
        )
        .entered();
        let zeros = [0; 64 * 1024];
        let mut logical_end = 0usize;
        for segment in &logical.segments {
            let start = segment.logical_offset.get::<byte>();
            let end = start + segment.length.get::<byte>();
            let zero_end = match segment.kind {
                crate::read::logical_read_plan::LogicalFileSegmentKind::Sparse => end,
                crate::read::logical_read_plan::LogicalFileSegmentKind::Physical { .. } => start,
            };
            if logical_end < zero_end {
                writer.seek(SeekFrom::Start(u64::try_from(logical_end)?))?;
                let mut remaining = zero_end - logical_end;
                while remaining > 0 {
                    let count = remaining.min(zeros.len());
                    writer.write_all(&zeros[..count])?;
                    remaining -= count;
                }
            }
            logical_end = logical_end.max(end);
        }
        let mut written = 0usize;
        PhysicalReader::try_new(source, physical, max_in_flight)?.read_each(|entry| {
            written += entry.write(writer)?;
            Ok(())
        })?;
        let expected = logical
            .physical_segments()
            .map(|segment| segment.length.get::<byte>())
            .sum::<usize>();
        eyre::ensure!(
            written == expected,
            "Incomplete streamed MFT: wrote {written} of {expected} physical bytes"
        );
        writer.seek(SeekFrom::Start(u64::try_from(size)?))?;
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use uom::si::information::byte;
    use uom::si::information::mebibyte;
    use uom::si::usize::Information;

    #[test]
    fn it_works() {
        assert_eq!(
            Information::new::<byte>(1_048_576).get::<byte>(),
            Information::new::<mebibyte>(1).get::<byte>()
        );
    }
}

#[cfg(test)]
mod streaming_tests {
    use super::*;
    use crate::mft::mft_file::MftFile;
    use crate::read::CancellableSeekWriter;
    use crate::read::logical_read_plan::LogicalFileSegment;
    use crate::read::logical_read_plan::LogicalFileSegmentKind;
    use std::io::Cursor;
    use std::io::Read;
    use teamy_cancellation::CancellationErrorExt;

    fn info(bytes: usize) -> Information {
        Information::new::<byte>(bytes)
    }

    #[test]
    fn writer_reassembles_fragmented_sparse_plan_and_zeroes_existing_gaps() -> eyre::Result<()> {
        let temp = tempfile::tempdir()?;
        let source_path = temp.path().join("source.bin");
        let source = (0..4096usize)
            .map(|offset| (offset % 251) as u8)
            .collect::<Vec<_>>();
        std::fs::write(&source_path, &source)?;
        let path = source_path.easy_pcwstr()?;
        let logical = LogicalReadPlan {
            segments: [
                LogicalFileSegment {
                    logical_offset: info(0),
                    length: info(768),
                    kind: LogicalFileSegmentKind::Physical {
                        physical_offset: info(2048),
                    },
                },
                LogicalFileSegment {
                    logical_offset: info(768),
                    length: info(512),
                    kind: LogicalFileSegmentKind::Sparse,
                },
                LogicalFileSegment {
                    logical_offset: info(1280),
                    length: info(1024),
                    kind: LogicalFileSegmentKind::Physical {
                        physical_offset: info(0),
                    },
                },
                LogicalFileSegment {
                    logical_offset: info(2304),
                    length: info(512),
                    kind: LogicalFileSegmentKind::Physical {
                        physical_offset: info(2048),
                    },
                },
                LogicalFileSegment {
                    logical_offset: info(3328),
                    length: info(256),
                    kind: LogicalFileSegmentKind::Sparse,
                },
            ]
            .into_iter()
            .collect(),
        };
        let mut physical = logical.as_physical_read_plan();
        physical.align_512();
        let physical = physical.chunked(Information::new::<mebibyte>(1));
        let mut expected = vec![0; 3584];
        expected[..768].copy_from_slice(&source[2048..2816]);
        expected[1280..2304].copy_from_slice(&source[..1024]);
        expected[2304..2816].copy_from_slice(&source[2048..2560]);
        let mut output = Cursor::new(vec![0xff; logical.total_logical_size().get::<byte>()]);
        output.set_position(17);

        PhysicalMftReadRequest::write_plan(physical, &path, &mut output)?;

        assert_eq!(output.position(), u64::try_from(expected.len())?);
        assert_eq!(output.get_ref(), &expected);
        Ok(())
    }

    #[test]
    fn regression_alignment_overlaps_are_read_once_with_production_chunks() -> eyre::Result<()> {
        let source = (0..1536usize)
            .map(|offset| u8::try_from(offset % 251).unwrap())
            .collect::<Vec<_>>();
        let mut source_file = tempfile::NamedTempFile::new()?;
        source_file.write_all(&source)?;
        source_file.flush()?;
        let path = source_file.path().easy_pcwstr()?;
        let logical = LogicalReadPlan {
            segments: [
                LogicalFileSegment {
                    logical_offset: info(0),
                    length: info(500),
                    kind: LogicalFileSegmentKind::Physical {
                        physical_offset: info(100),
                    },
                },
                LogicalFileSegment {
                    logical_offset: info(500),
                    length: info(500),
                    kind: LogicalFileSegmentKind::Physical {
                        physical_offset: info(600),
                    },
                },
            ]
            .into_iter()
            .collect(),
        };
        let mut physical = logical.as_physical_read_plan();
        physical.align_512();
        let physical = physical.chunked(Information::new::<mebibyte>(1));
        let mut output = Cursor::new(vec![0xff; 1000]);

        PhysicalMftReadRequest::write_plan(physical, &path, &mut output)?;

        assert_eq!(output.position(), 1000);
        assert_eq!(output.get_ref(), &source[100..1100]);
        Ok(())
    }

    #[test]
    fn all_sparse_output_is_zeroed_without_relying_on_file_allocation() -> eyre::Result<()> {
        let source = tempfile::NamedTempFile::new()?;
        let path = source.path().easy_pcwstr()?;
        let logical = LogicalReadPlan {
            segments: [LogicalFileSegment {
                logical_offset: info(128),
                length: info(256),
                kind: LogicalFileSegmentKind::Sparse,
            }]
            .into_iter()
            .collect(),
        };
        let mut output = Cursor::new(vec![0xff; 384]);
        PhysicalMftReadRequest::write_plan(logical.as_physical_read_plan(), &path, &mut output)?;
        assert_eq!(output.into_inner(), vec![0; 384]);
        Ok(())
    }

    #[test]
    fn short_read_returns_an_error_to_the_writer_owner() -> eyre::Result<()> {
        let source = tempfile::NamedTempFile::new()?;
        source.as_file().set_len(512)?;
        let path = source.path().easy_pcwstr()?;
        let logical = LogicalReadPlan {
            segments: [LogicalFileSegment {
                logical_offset: info(0),
                length: info(1024),
                kind: LogicalFileSegmentKind::Physical {
                    physical_offset: info(0),
                },
            }]
            .into_iter()
            .collect(),
        };
        let mut output = Cursor::new(Vec::new());
        let error =
            PhysicalMftReadRequest::write_plan(logical.as_physical_read_plan(), &path, &mut output)
                .unwrap_err();
        assert!(error.to_string().contains("Short physical read"));
        assert!(output.into_inner().is_empty());
        Ok(())
    }

    #[test]
    fn cancellation_is_observed_through_the_supplied_writer() -> eyre::Result<()> {
        let source = tempfile::NamedTempFile::new()?;
        source.as_file().set_len(512)?;
        let path = source.path().easy_pcwstr()?;
        let logical = LogicalReadPlan {
            segments: [LogicalFileSegment {
                logical_offset: info(0),
                length: info(512),
                kind: LogicalFileSegmentKind::Physical {
                    physical_offset: info(0),
                },
            }]
            .into_iter()
            .collect(),
        };
        let mut output = Cursor::new(vec![0xff; 512]);
        let token = crate::cancellation::CancellationToken::new();
        let error = token
            .with_scope(|scope| {
                token.request_cancel("cancel through writer");
                let mut writer = CancellableSeekWriter::new(&mut output, &scope);
                PhysicalMftReadRequest::write_plan(
                    logical.as_physical_read_plan(),
                    &path,
                    &mut writer,
                )
            })
            .unwrap_err();
        assert_eq!(
            error
                .as_cancellation()
                .expect("writer must preserve typed cancellation")
                .reason,
            "cancel through writer"
        );
        assert_eq!(output.into_inner(), vec![0xff; 512]);
        Ok(())
    }

    #[test]
    fn fragmented_valid_records_preserve_paths_and_deleted_flags() -> eyre::Result<()> {
        let temp = tempfile::tempdir()?;
        let raw = crate::mft::sample_mft_records();
        let split = 4096;
        let physical_bytes = [raw[split..].to_vec(), raw[..split].to_vec()].concat();
        let source = temp.path().join("source.bin");
        std::fs::write(&source, physical_bytes)?;
        let path = source.easy_pcwstr()?;
        let logical = LogicalReadPlan {
            segments: [
                LogicalFileSegment {
                    logical_offset: info(0),
                    length: info(split),
                    kind: LogicalFileSegmentKind::Physical {
                        physical_offset: info(raw.len() - split),
                    },
                },
                LogicalFileSegment {
                    logical_offset: info(split),
                    length: info(raw.len() - split),
                    kind: LogicalFileSegmentKind::Physical {
                        physical_offset: info(0),
                    },
                },
            ]
            .into_iter()
            .collect(),
        };
        let physical = logical.as_physical_read_plan().chunked(info(512));
        let mut output = Cursor::new(Vec::new());
        PhysicalMftReadRequest::write_plan(physical, &path, &mut output)?;
        assert_eq!(output.get_ref(), &raw);
        let streamed = MftFile::from_vec(output.into_inner())?;
        let paths =
            crate::mft::mft_convert_to_path_collection::convert_mft_file_to_path_collection(
                DriveLetter::C,
                &streamed,
            )?;
        let rows = paths
            .0
            .into_iter()
            .flatten()
            .map(|path| {
                (
                    path.path.to_string_lossy().into_owned(),
                    path.has_deleted_entries(),
                )
            })
            .collect::<Vec<_>>();
        assert!(rows.contains(&(r"C:\folder\example.txt".into(), false)));
        assert!(rows.contains(&(r"C:\linked.txt".into(), false)));
        assert!(rows.contains(&(r"C:\folder\gone.txt".into(), true)));
        Ok(())
    }

    #[test]
    fn completed_entries_keep_the_plan_after_the_reader_is_dropped() -> eyre::Result<()> {
        let mut source = tempfile::NamedTempFile::new()?;
        source.write_all(&[b'A'; 512])?;
        source.write_all(&[b'B'; 512])?;
        source.flush()?;
        let path = source.path().easy_pcwstr()?;
        let logical = LogicalReadPlan {
            segments: [
                LogicalFileSegment {
                    logical_offset: info(0),
                    length: info(512),
                    kind: LogicalFileSegmentKind::Physical {
                        physical_offset: info(512),
                    },
                },
                LogicalFileSegment {
                    logical_offset: info(512),
                    length: info(512),
                    kind: LogicalFileSegmentKind::Physical {
                        physical_offset: info(0),
                    },
                },
            ]
            .into_iter()
            .collect(),
        };
        let mut entries = Vec::new();
        PhysicalReader::try_new(&path, logical.as_physical_read_plan(), 2.into())?.read_each(
            |entry| {
                entries.push(entry);
                Ok(())
            },
        )?;
        assert_eq!(entries.len(), 2);

        let mut output = Cursor::new(vec![0xff; 1024]);
        for entry in entries {
            assert_eq!(entry.write(&mut output)?, 512);
        }
        assert_eq!(&output.get_ref()[..512], &[b'B'; 512]);
        assert_eq!(&output.get_ref()[512..], &[b'A'; 512]);
        Ok(())
    }

    #[test]
    fn streaming_reader_bounds_retention_and_drains_on_consumer_failure() -> eyre::Result<()> {
        let mut source = tempfile::NamedTempFile::new()?;
        source.write_all(&vec![7; 4 * 1024 * 1024])?;
        source.flush()?;
        let path = source.path().to_string_lossy().easy_pcwstr()?;
        let logical = LogicalReadPlan {
            segments: [LogicalFileSegment {
                logical_offset: info(0),
                length: info(4 * 1024 * 1024),
                kind: LogicalFileSegmentKind::Physical {
                    physical_offset: info(0),
                },
            }]
            .into_iter()
            .collect(),
        };
        let plan = logical.as_physical_read_plan().chunked(info(65536));
        let mut count = 0;
        let mut bytes = 0;
        PhysicalReader::try_new(&path, plan.clone(), 2.into())?.read_each(|entry| {
            assert_eq!(entry.data().len(), 65536);
            assert!(entry.data().iter().all(|b| *b == 7));
            count += 1;
            bytes += entry.data().len();
            Ok(())
        })?;
        assert_eq!(count, 64);
        assert_eq!(bytes, 4 * 1024 * 1024);
        let error = PhysicalReader::try_new(&path, plan, 2.into())?
            .read_each(|_| eyre::bail!("injected writer failure"));
        assert!(
            error
                .unwrap_err()
                .to_string()
                .contains("injected writer failure")
        );
        // Reader drop has drained pending I/O; source remains usable.
        source.seek(SeekFrom::Start(0))?;
        let mut probe = [0];
        source.read_exact(&mut probe)?;
        assert_eq!(probe, [7]);
        Ok(())
    }

    #[test]
    fn consumer_panic_drains_accepted_reads_before_the_source_is_reused() -> eyre::Result<()> {
        let mut source = tempfile::NamedTempFile::new()?;
        let mut expected = vec![0; 6 * 512];
        for (index, chunk) in expected.chunks_mut(512).enumerate() {
            chunk.fill(b'A' + u8::try_from(index)?);
        }
        source.write_all(&expected)?;
        source.flush()?;
        let path = source.path().easy_pcwstr()?;
        let logical = LogicalReadPlan {
            segments: [LogicalFileSegment {
                logical_offset: info(0),
                length: info(expected.len()),
                kind: LogicalFileSegmentKind::Physical {
                    physical_offset: info(0),
                },
            }]
            .into_iter()
            .collect(),
        };
        let plan = logical.as_physical_read_plan().chunked(info(512));
        let reader = PhysicalReader::try_new(&path, plan.clone(), 3.into())?;

        // The first callback runs after three reads are submitted and only one is received.
        let panic =
            std::panic::catch_unwind(|| reader.read_each(|_| panic!("injected consumer panic")))
                .expect_err("the consumer panic must unwind through the reader");
        assert_eq!(
            panic.downcast_ref::<&str>(),
            Some(&"injected consumer panic")
        );

        source.seek(SeekFrom::Start(0))?;
        let mut reread = Vec::new();
        source.read_to_end(&mut reread)?;
        assert_eq!(reread, expected);

        let mut output = Cursor::new(vec![0; expected.len()]);
        PhysicalReader::try_new(&path, plan, 3.into())?.read_each(|entry| {
            entry.write(&mut output)?;
            Ok(())
        })?;
        assert_eq!(output.into_inner(), expected);
        Ok(())
    }

    // Run separately so an external process monitor can inspect streaming retention.
    #[test]
    #[ignore = "manual bounded 64 MiB ordinary-file memory measurement"]
    fn measure_streaming_reader() -> eyre::Result<()> {
        let mut source = tempfile::NamedTempFile::new()?;
        let block = vec![3; 1024 * 1024];
        for _ in 0..64 {
            source.write_all(&block)?;
        }
        source.flush()?;
        drop(block);
        let path = source.path().to_string_lossy().easy_pcwstr()?;
        let logical = LogicalReadPlan {
            segments: [LogicalFileSegment {
                logical_offset: info(0),
                length: info(64 * 1024 * 1024),
                kind: LogicalFileSegmentKind::Physical {
                    physical_offset: info(0),
                },
            }]
            .into_iter()
            .collect(),
        };
        let plan = logical.as_physical_read_plan().chunked(info(1024 * 1024));
        let start = std::time::Instant::now();
        let mut count = 0;
        let mut total = 0;
        PhysicalReader::try_new(&path, plan, 2.into())?.read_each(|entry| {
            assert_eq!(entry.data().len(), 1024 * 1024);
            assert!(entry.data().iter().all(|value| *value == 3));
            count += 1;
            total += entry.data().len();
            Ok(())
        })?;
        assert_eq!(count, 64);
        assert_eq!(total, 64 * 1024 * 1024);
        std::thread::sleep(std::time::Duration::from_millis(250));
        println!(
            "streaming bytes=67108864 io_depth=2 elapsed_ms={} (includes 250 ms monitoring hold)",
            start.elapsed().as_millis()
        );
        Ok(())
    }
}
