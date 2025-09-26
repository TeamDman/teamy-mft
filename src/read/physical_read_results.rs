use crate::read::logical_read_plan::LogicalReadPlan;
use crate::read::physical_read_request::PhysicalReadRequest;
use std::collections::BTreeSet;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::ops::Bound;
use tracing::debug;
use uom::si::information::byte;
use uom::si::usize::Information;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PhysicalReadResultEntry {
    pub request: PhysicalReadRequest,
    pub data: Vec<u8>,
}

impl Ord for PhysicalReadResultEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.request.cmp(&other.request)
    }
}
impl PartialOrd for PhysicalReadResultEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug)]
pub struct PhysicalReadResults {
    pub entries: BTreeSet<PhysicalReadResultEntry>,
}
impl PhysicalReadResults {
    pub fn new() -> Self {
        Self {
            entries: BTreeSet::new(),
        }
    }

    /// Consumes the results and writes them to a file (pre-sizing & zero-filling gaps by allocation).
    pub fn write_to_file(
        &self,
        logical_plan: &LogicalReadPlan,
        output_path: impl AsRef<std::path::Path>,
    ) -> eyre::Result<()> {
        debug!("Writing MFT output to {:?}", output_path.as_ref());
        let file = std::fs::File::create(output_path)?;
        file.set_len(logical_plan.total_logical_size().get::<byte>() as u64)?;

        let mut writer = std::io::BufWriter::new(file);
        // writer.seek(SeekFrom::Start(logical_offset))?;
        // writer.write_all(slice)?;

        debug!("Writing {} logical segments", logical_plan.segments.len());
        for logical_segment in logical_plan.segments.iter() {
            let Some(physical_segment) = logical_segment.as_physical_read_request() else {
                // Sparse segment, skip
                continue;
            };
            // A given logical segment may have been split into multiple physical reads.
            let mut physical_offset_current = physical_segment.offset;
            let physical_offset_end = physical_segment.offset + physical_segment.length;

            debug!(
                ?logical_segment,
                "Identifying physical data for logical segment"
            );
            while physical_offset_current < physical_offset_end {
                debug!(
                    physical_offset_current = physical_offset_current.get::<byte>(),
                    physical_offset_end = physical_offset_end.get::<byte>(),
                    remaining = (physical_offset_end - physical_offset_current).get_human(),
                    "Locating physical data for logical segment",
                );

                // Identify the entry that contains this offset
                // Find the entry containing this offset. Use lower_bound and if it overshoots,
                // step back to the predecessor and verify containment.
                let mut cursor = self
                    .entries
                    .lower_bound(Bound::Included(&PhysicalReadResultEntry {
                        request: PhysicalReadRequest::new(
                            physical_offset_current,
                            Information::new::<byte>(1),
                        ),
                        data: vec![],
                    }));
                let mut entry = cursor.next();
                if let Some(e) = entry {
                    if e.request.offset > physical_offset_current {
                        entry = cursor.prev();
                    }
                } else {
                    // lower_bound returned end; try the last element
                    entry = self.entries.last();
                }
                let Some(entry) = entry else {
                    eyre::bail!("Missing physical read data at offset {physical_offset_current:?} - no entries available");
                };
                if !(entry.request.offset <= physical_offset_current
                    && physical_offset_current < entry.request.offset + entry.request.length)
                {
                    eyre::bail!("Missing physical read data at offset {physical_offset_current:?} - not contained in any entry");
                }

                // Identify what part of this entry to write
                let offset_within_entry = physical_offset_current - entry.request.offset;
                let bytes_available = entry.request.length - offset_within_entry;
                let bytes_needed = physical_offset_end - physical_offset_current;
                let bytes_to_write = if bytes_available < bytes_needed {
                    bytes_available
                } else {
                    bytes_needed
                };
                let slice = &entry.data[offset_within_entry.get::<byte>()
                    ..(offset_within_entry + bytes_to_write).get::<byte>()];

                debug!(
                    ?entry.request,
                    offset_within_entry = offset_within_entry.get::<byte>(),
                    bytes_to_write = bytes_to_write.get::<byte>(),
                    physical_offset_current = physical_offset_current.get::<byte>(),
                    "Writing physical data for logical segment",
                );

                // Write it
                writer.seek(SeekFrom::Start(
                    logical_segment.logical_offset.get::<byte>() as u64
                        + (physical_offset_current - physical_segment.offset).get::<byte>() as u64,
                ))?;
                writer.write_all(slice)?;

                // Advance
                physical_offset_current += bytes_to_write;
            }
        }

        writer.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::read::logical_read_plan::LogicalFileSegment;
    use crate::read::logical_read_plan::LogicalFileSegmentKind;
    use crate::read::logical_read_plan::LogicalReadPlan;
    use crate::read::physical_read_request::PhysicalReadRequest;
    use crate::read::physical_read_results::PhysicalReadResultEntry;
    use crate::read::physical_read_results::PhysicalReadResults;
    use uom::si::information::byte;
    use uom::si::usize::Information;

    #[test]
    fn writes_blocks_and_preserves_gap_zero() -> eyre::Result<()> {
        let temp = tempfile::NamedTempFile::new().expect("tmp");
        let path = temp.path().to_path_buf();
        // Two blocks with a gap in between
        let read_plan = LogicalReadPlan {
            segments: [
                LogicalFileSegment {
                    logical_offset: Information::new::<byte>(0),
                    length: Information::new::<byte>(4),
                    kind: LogicalFileSegmentKind::Physical {
                        physical_offset: Information::new::<byte>(0),
                    },
                },
                LogicalFileSegment {
                    logical_offset: Information::new::<byte>(10),
                    length: Information::new::<byte>(3),
                    kind: LogicalFileSegmentKind::Physical {
                        physical_offset: Information::new::<byte>(1000),
                    },
                },
            ]
            .into_iter()
            .collect(),
        };
        let read_results = PhysicalReadResults {
            entries: [
                PhysicalReadResultEntry {
                    request: PhysicalReadRequest {
                        offset: Information::new::<byte>(0),
                        length: Information::new::<byte>(4),
                    },
                    data: b"ABCD".to_vec(),
                },
                PhysicalReadResultEntry {
                    request: PhysicalReadRequest {
                        offset: Information::new::<byte>(1000),
                        length: Information::new::<byte>(3),
                    },
                    data: b"XYZ".to_vec(),
                },
            ]
            .into_iter()
            .collect(),
        };

        read_results.write_to_file(&read_plan, &path)?;
        let bytes = std::fs::read(&path).unwrap();
        // The file is pre-sized to the total logical size: 4 + 6 gap + 3 = 13
        assert_eq!(bytes.len(), 13);
        assert_eq!(&bytes[0..4], b"ABCD");
        assert_eq!(&bytes[4..10], &[0u8; 6]); // gap zeroed
        assert_eq!(&bytes[10..13], b"XYZ");
        Ok(())
    }

    #[test]
    fn writes_from_predecessor_when_aligned_overread() -> eyre::Result<()> {
        // Logical segment expects data at physical offset 100 of length 10.
        // But the actual physical read was aligned earlier, starting at 64, length 64 (covering 64..128).
        // The current implementation using lower_bound without predecessor check fails to find data at 100.
        let temp = tempfile::NamedTempFile::new().expect("tmp");
        let path = temp.path().to_path_buf();

        let read_plan = LogicalReadPlan {
            segments: [LogicalFileSegment {
                logical_offset: Information::new::<byte>(0),
                length: Information::new::<byte>(10),
                kind: LogicalFileSegmentKind::Physical {
                    physical_offset: Information::new::<byte>(100),
                },
            }]
            .into_iter()
            .collect(),
        };

        // Provide 64 bytes starting at 64, containing bytes 64..127.
        let mut data = vec![0u8; 64];
        for (i, b) in data.iter_mut().enumerate() {
            *b = (64 + i as u8) as u8; // distinct content to verify slice is correct
        }
        let read_results = PhysicalReadResults {
            entries: [PhysicalReadResultEntry {
                request: PhysicalReadRequest {
                    offset: Information::new::<byte>(64),
                    length: Information::new::<byte>(64),
                },
                data,
            }]
            .into_iter()
            .collect(),
        };

        // Expect write to succeed and produce 10 bytes taken from within the aligned block starting at 100.
        // Specifically, bytes 100..110 correspond to indices 36..46 within the data above.
        read_results.write_to_file(&read_plan, &path)?;
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.len(), 10);
        for (i, b) in bytes.iter().enumerate() {
            assert_eq!(*b as usize, 100 + i);
        }
        Ok(())
    }
}
