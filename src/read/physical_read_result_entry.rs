use crate::read::logical_read_plan::LogicalReadPlan;
use crate::read::physical_read_request::PhysicalReadRequest;
use eyre::ContextCompat;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use uom::si::information::byte;

/// One completed physical read, borrowing the plan that determines its logical destinations.
///
/// The entry cannot outlive that plan:
///
/// ```compile_fail
/// use teamy_mft::read::physical_read_result_entry::PhysicalReadResultEntry;
///
/// fn escape(entry: PhysicalReadResultEntry<'_>) -> PhysicalReadResultEntry<'static> {
///     entry
/// }
/// ```
#[derive(Debug, Clone)]
pub struct PhysicalReadResultEntry<'plan> {
    logical_plan: &'plan LogicalReadPlan,
    request: PhysicalReadRequest,
    data: Vec<u8>,
}

impl<'plan> PhysicalReadResultEntry<'plan> {
    /// # Errors
    /// Returns an error if the buffer length differs from the requested range length.
    pub(super) fn try_new(
        logical_plan: &'plan LogicalReadPlan,
        request: PhysicalReadRequest,
        data: Vec<u8>,
    ) -> eyre::Result<Self> {
        let expected_len = request.range().length().get::<byte>();
        eyre::ensure!(
            data.len() == expected_len,
            "Physical read buffer length mismatch: expected {expected_len} bytes, received {}",
            data.len()
        );
        Ok(Self {
            logical_plan,
            request,
            data,
        })
    }

    #[must_use]
    pub fn request(&self) -> &PhysicalReadRequest {
        &self.request
    }

    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Write the read's intersections with every physical segment in its logical plan.
    ///
    /// # Errors
    ///
    /// Returns an error if a physical or logical range overflows,
    /// or the writer cannot seek or accept all bytes.
    pub fn write(&self, writer: &mut (impl Seek + Write)) -> eyre::Result<usize> {
        let this_physical_request_range = self.request.range();
        let mut written = 0;
        for logical_segment in &self.logical_plan.segments {
            let Some(logical_segment_physical_range) = logical_segment.physical_range()? else {
                continue;
            };
            let Some(physical_coverage) =
                logical_segment_physical_range.intersection(&this_physical_request_range)
            else {
                continue;
            };
            let this_result_data_range =
                physical_coverage.relative_to(&this_physical_request_range)?;
            let logical_destination_range = physical_coverage
                .relative_to(&logical_segment_physical_range)?
                .shifted_by(logical_segment.logical_range.start())?;
            let data = self
                .data
                .get(this_result_data_range.byte_indices())
                .wrap_err("Physical read intersection exceeds its completed buffer")?;
            writer.seek(SeekFrom::Start(u64::try_from(
                logical_destination_range.start().get::<byte>(),
            )?))?;
            writer.write_all(data)?;
            written += data.len();
        }
        Ok(written)
    }
}

#[cfg(test)]
mod tests {
    use super::PhysicalReadResultEntry;
    use crate::read::byte_range::ByteRange;
    use crate::read::logical_read_plan::LogicalFileSegment;
    use crate::read::logical_read_plan::LogicalFileSegmentKind;
    use crate::read::logical_read_plan::LogicalReadPlan;
    use crate::read::physical_read_request::PhysicalReadRequest;
    use std::collections::BTreeSet;
    use std::io;
    use std::io::Cursor;
    use std::io::Seek;
    use std::io::SeekFrom;
    use std::io::Write;
    use uom::si::information::byte;
    use uom::si::usize::Information;

    fn info(bytes: usize) -> Information {
        Information::new::<byte>(bytes)
    }

    #[test]
    fn aligned_read_maps_intersections_to_multiple_logical_destinations() -> eyre::Result<()> {
        let logical = LogicalReadPlan {
            segments: [
                LogicalFileSegment {
                    logical_range: ByteRange::from_offset_and_length(info(0), info(256))?,
                    kind: LogicalFileSegmentKind::Physical {
                        physical_offset: info(768),
                    },
                },
                LogicalFileSegment {
                    logical_range: ByteRange::from_offset_and_length(info(256), info(256))?,
                    kind: LogicalFileSegmentKind::Sparse,
                },
                LogicalFileSegment {
                    logical_range: ByteRange::from_offset_and_length(info(512), info(512))?,
                    kind: LogicalFileSegmentKind::Physical {
                        physical_offset: info(512),
                    },
                },
                LogicalFileSegment {
                    logical_range: ByteRange::from_offset_and_length(info(1024), info(256))?,
                    kind: LogicalFileSegmentKind::Physical {
                        physical_offset: info(768),
                    },
                },
            ]
            .into_iter()
            .collect(),
        };
        let data = (0..512)
            .map(|index| u8::try_from(index % 251).unwrap())
            .collect::<Vec<_>>();
        let entry = PhysicalReadResultEntry::try_new(
            &logical,
            PhysicalReadRequest::from(ByteRange::from_offset_and_length(info(512), info(512))?),
            data.clone(),
        )?;
        let mut output = Cursor::new(vec![0xff; 1280]);
        let mut expected = vec![0xff; 1280];
        expected[..256].copy_from_slice(&data[256..512]);
        expected[512..1024].copy_from_slice(&data[..512]);
        expected[1024..].copy_from_slice(&data[256..512]);

        assert_eq!(entry.write(&mut output)?, 1024);
        assert_eq!(output.into_inner(), expected);
        assert_eq!(entry.request().range().start(), info(512));
        assert_eq!(entry.data(), data.as_slice());
        Ok(())
    }

    #[test]
    fn aligned_read_writes_only_the_overlapping_part_of_a_segment() -> eyre::Result<()> {
        let logical = LogicalReadPlan {
            segments: [LogicalFileSegment {
                logical_range: ByteRange::from_offset_and_length(info(0), info(200))?,
                kind: LogicalFileSegmentKind::Physical {
                    physical_offset: info(400),
                },
            }]
            .into_iter()
            .collect(),
        };
        let entry = PhysicalReadResultEntry::try_new(
            &logical,
            PhysicalReadRequest::from(ByteRange::from_offset_and_length(info(0), info(512))?),
            vec![7; 512],
        )?;
        let mut output = Cursor::new(vec![0xff; 200]);

        assert_eq!(entry.write(&mut output)?, 112);
        assert_eq!(&output.get_ref()[..112], &[7; 112]);
        assert_eq!(&output.get_ref()[112..], &[0xff; 88]);
        Ok(())
    }

    #[test]
    fn regression_high_physical_offset_maps_to_small_logical_offset() -> eyre::Result<()> {
        let logical = LogicalReadPlan {
            segments: [LogicalFileSegment {
                logical_range: ByteRange::from_offset_and_length(info(512), info(1))?,
                kind: LogicalFileSegmentKind::Physical {
                    physical_offset: info(usize::MAX - 511),
                },
            }]
            .into_iter()
            .collect(),
        };
        let entry = PhysicalReadResultEntry::try_new(
            &logical,
            PhysicalReadRequest::from(ByteRange::from_offset_and_length(
                info(usize::MAX - 511),
                info(1),
            )?),
            vec![7],
        )?;
        let mut output = Cursor::new(vec![0xff; 513]);

        assert_eq!(entry.write(&mut output)?, 1);
        assert_eq!(output.position(), 513);
        assert_eq!(&output.get_ref()[..512], &[0xff; 512]);
        assert_eq!(output.get_ref()[512], 7);
        Ok(())
    }

    #[test]
    fn constructor_rejects_mismatched_buffer_lengths() -> eyre::Result<()> {
        let logical = LogicalReadPlan {
            segments: BTreeSet::new(),
        };
        let request =
            PhysicalReadRequest::from(ByteRange::from_offset_and_length(info(0), info(512))?);
        for length in [0, 511, 513] {
            let error =
                PhysicalReadResultEntry::try_new(&logical, request, vec![7; length]).unwrap_err();
            assert_eq!(
                error.to_string(),
                format!(
                    "Physical read buffer length mismatch: expected 512 bytes, received {length}"
                )
            );
        }
        Ok(())
    }

    #[test]
    fn disjoint_and_touching_segments_leave_output_unchanged() -> eyre::Result<()> {
        let logical = LogicalReadPlan {
            segments: [
                LogicalFileSegment {
                    logical_range: ByteRange::from_offset_and_length(info(0), info(100))?,
                    kind: LogicalFileSegmentKind::Physical {
                        physical_offset: info(100),
                    },
                },
                LogicalFileSegment {
                    logical_range: ByteRange::from_offset_and_length(info(100), info(100))?,
                    kind: LogicalFileSegmentKind::Physical {
                        physical_offset: info(300),
                    },
                },
                LogicalFileSegment {
                    logical_range: ByteRange::from_offset_and_length(info(200), info(100))?,
                    kind: LogicalFileSegmentKind::Physical {
                        physical_offset: info(500),
                    },
                },
            ]
            .into_iter()
            .collect(),
        };
        let entry = PhysicalReadResultEntry::try_new(
            &logical,
            PhysicalReadRequest::from(ByteRange::from_offset_and_length(info(200), info(100))?),
            vec![7; 100],
        )?;
        let mut output = Cursor::new(b"unchanged".to_vec());
        output.set_position(3);

        assert_eq!(entry.write(&mut output)?, 0);
        assert_eq!(output.position(), 3);
        assert_eq!(output.into_inner(), b"unchanged");
        Ok(())
    }

    struct FailingWriter {
        fail_seek: bool,
    }

    impl Seek for FailingWriter {
        fn seek(&mut self, _position: SeekFrom) -> io::Result<u64> {
            if self.fail_seek {
                Err(io::Error::new(io::ErrorKind::InvalidInput, "seek failure"))
            } else {
                Ok(0)
            }
        }
    }

    impl Write for FailingWriter {
        fn write(&mut self, _bytes: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "write failure"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn writer_errors_preserve_the_original_error_kind() -> eyre::Result<()> {
        let logical = LogicalReadPlan {
            segments: [LogicalFileSegment {
                logical_range: ByteRange::from_offset_and_length(info(0), info(512))?,
                kind: LogicalFileSegmentKind::Physical {
                    physical_offset: info(0),
                },
            }]
            .into_iter()
            .collect(),
        };
        let entry = PhysicalReadResultEntry::try_new(
            &logical,
            PhysicalReadRequest::from(ByteRange::from_offset_and_length(info(0), info(512))?),
            vec![7; 512],
        )?;
        for (fail_seek, expected) in [
            (true, io::ErrorKind::InvalidInput),
            (false, io::ErrorKind::BrokenPipe),
        ] {
            let error = entry.write(&mut FailingWriter { fail_seek }).unwrap_err();
            assert_eq!(error.downcast_ref::<io::Error>().unwrap().kind(), expected);
        }
        Ok(())
    }
}
