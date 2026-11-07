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
    pub(super) fn new(
        logical_plan: &'plan LogicalReadPlan,
        request: PhysicalReadRequest,
        data: Vec<u8>,
    ) -> Self {
        Self {
            logical_plan,
            request,
            data,
        }
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
    /// Returns an error if the read is short, a logical offset cannot be represented,
    /// or the writer cannot seek or accept all bytes.
    pub fn write(&self, writer: &mut (impl Seek + Write)) -> eyre::Result<usize> {
        eyre::ensure!(
            self.data.len() == self.request.length.get::<byte>(),
            "Short physical read"
        );
        let mut written = 0;
        for segment in self.logical_plan.physical_segments() {
            let Some(source) = segment.as_physical_read_request() else {
                continue;
            };
            let start = std::cmp::max(source.offset, self.request.offset);
            let end = std::cmp::min(source.physical_end(), self.request.physical_end());
            if start >= end {
                continue;
            }
            let offset = (start - self.request.offset).get::<byte>();
            let length = (end - start).get::<byte>();
            let source_relative_offset = (start - source.offset).get::<byte>();
            let logical_offset = segment
                .logical_offset
                .get::<byte>()
                .checked_add(source_relative_offset)
                .wrap_err("Logical write offset overflows")?;
            logical_offset
                .checked_add(length)
                .wrap_err("Logical write range overflows")?;
            writer.seek(SeekFrom::Start(u64::try_from(logical_offset)?))?;
            writer.write_all(&self.data[offset..offset + length])?;
            written += length;
        }
        Ok(written)
    }
}

#[cfg(test)]
mod tests {
    use super::PhysicalReadResultEntry;
    use crate::read::logical_read_plan::LogicalFileSegment;
    use crate::read::logical_read_plan::LogicalFileSegmentKind;
    use crate::read::logical_read_plan::LogicalReadPlan;
    use crate::read::physical_read_request::PhysicalReadRequest;
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
                    logical_offset: info(0),
                    length: info(256),
                    kind: LogicalFileSegmentKind::Physical {
                        physical_offset: info(768),
                    },
                },
                LogicalFileSegment {
                    logical_offset: info(256),
                    length: info(256),
                    kind: LogicalFileSegmentKind::Sparse,
                },
                LogicalFileSegment {
                    logical_offset: info(512),
                    length: info(512),
                    kind: LogicalFileSegmentKind::Physical {
                        physical_offset: info(512),
                    },
                },
                LogicalFileSegment {
                    logical_offset: info(1024),
                    length: info(256),
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
        let entry = PhysicalReadResultEntry::new(
            &logical,
            PhysicalReadRequest::new(info(512), info(512)),
            data.clone(),
        );
        let mut output = Cursor::new(vec![0xff; 1280]);
        let mut expected = vec![0xff; 1280];
        expected[..256].copy_from_slice(&data[256..512]);
        expected[512..1024].copy_from_slice(&data[..512]);
        expected[1024..].copy_from_slice(&data[256..512]);

        assert_eq!(entry.write(&mut output)?, 1024);
        assert_eq!(output.into_inner(), expected);
        assert_eq!(entry.request().offset, info(512));
        assert_eq!(entry.data(), data.as_slice());
        Ok(())
    }

    #[test]
    fn aligned_read_writes_only_the_overlapping_part_of_a_segment() -> eyre::Result<()> {
        let logical = LogicalReadPlan {
            segments: [LogicalFileSegment {
                logical_offset: info(0),
                length: info(200),
                kind: LogicalFileSegmentKind::Physical {
                    physical_offset: info(400),
                },
            }]
            .into_iter()
            .collect(),
        };
        let entry = PhysicalReadResultEntry::new(
            &logical,
            PhysicalReadRequest::new(info(0), info(512)),
            vec![7; 512],
        );
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
                logical_offset: info(512),
                length: info(1),
                kind: LogicalFileSegmentKind::Physical {
                    physical_offset: info(usize::MAX - 511),
                },
            }]
            .into_iter()
            .collect(),
        };
        let entry = PhysicalReadResultEntry::new(
            &logical,
            PhysicalReadRequest::new(info(usize::MAX - 511), info(1)),
            vec![7],
        );
        let mut output = Cursor::new(vec![0xff; 513]);

        assert_eq!(entry.write(&mut output)?, 1);
        assert_eq!(output.position(), 513);
        assert_eq!(&output.get_ref()[..512], &[0xff; 512]);
        assert_eq!(output.get_ref()[512], 7);
        Ok(())
    }

    #[test]
    fn regression_logical_write_overflow_preserves_output() {
        let logical = LogicalReadPlan {
            segments: [LogicalFileSegment {
                logical_offset: info(usize::MAX - 1),
                length: info(2),
                kind: LogicalFileSegmentKind::Physical {
                    physical_offset: info(0),
                },
            }]
            .into_iter()
            .collect(),
        };
        let entry = PhysicalReadResultEntry::new(
            &logical,
            PhysicalReadRequest::new(info(0), info(2)),
            vec![7; 2],
        );
        let mut output = Cursor::new(b"unchanged".to_vec());
        output.set_position(3);

        let error = entry.write(&mut output).unwrap_err();
        assert_eq!(error.to_string(), "Logical write range overflows");
        assert_eq!(output.position(), 3);
        assert_eq!(output.into_inner(), b"unchanged");
    }

    #[test]
    fn short_read_is_rejected_before_output_is_changed() {
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
        let entry = PhysicalReadResultEntry::new(
            &logical,
            PhysicalReadRequest::new(info(0), info(512)),
            vec![7; 511],
        );
        let mut output = Cursor::new(b"unchanged".to_vec());
        output.set_position(3);

        assert!(entry.write(&mut output).is_err());
        assert_eq!(output.position(), 3);
        assert_eq!(output.into_inner(), b"unchanged");
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
    fn writer_errors_preserve_the_original_error_kind() {
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
        let entry = PhysicalReadResultEntry::new(
            &logical,
            PhysicalReadRequest::new(info(0), info(512)),
            vec![7; 512],
        );
        for (fail_seek, expected) in [
            (true, io::ErrorKind::InvalidInput),
            (false, io::ErrorKind::BrokenPipe),
        ] {
            let error = entry.write(&mut FailingWriter { fail_seek }).unwrap_err();
            assert_eq!(error.downcast_ref::<io::Error>().unwrap().kind(), expected);
        }
    }
}
