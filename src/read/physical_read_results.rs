use crate::read::logical_read_plan::LogicalFileSegment;
use crate::read::logical_read_plan::LogicalReadPlan;
use crate::read::physical_read_request::PhysicalReadRequest;
use humansize::BINARY;
use std::collections::BTreeSet;
use std::io::Cursor;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use teamy_uom_extensions::HumanInformationExt;
#[cfg(feature = "tracy")]
use tracing::debug_span;
use tracing::info_span;
use tracing::debug;
use tracing::trace;
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
    pub entries: BTreeSet<PhysicalReadResultEntry>, // TODO: replace with masstree
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PhysicalReadResultsIterValue<'a> {
    /// Destination logical file offset where this chunk should be written.
    pub logical_offset: Information,
    /// Source physical device offset this chunk came from.
    pub physical_offset: Information,
    /// Borrowed data chunk to be written at `logical_offset`.
    pub bytes: &'a [u8],
}

impl PhysicalReadResultsIterValue<'_> {
    #[must_use]
    pub fn length(&self) -> Information {
        Information::new::<byte>(self.bytes.len())
    }
}

#[derive(Debug, Clone, Copy)]
struct ActivePhysicalSegment {
    logical_offset_start: Information,
    physical_offset_start: Information,
    physical_offset_current: Information,
    physical_offset_end: Information,
}

#[derive(Debug)]
pub struct PhysicalReadResultsIter<'a> {
    entries: &'a BTreeSet<PhysicalReadResultEntry>,
    logical_segments: std::collections::btree_set::Iter<'a, LogicalFileSegment>,
    active_segment: Option<ActivePhysicalSegment>,
    done: bool,
}

impl Default for PhysicalReadResults {
    fn default() -> Self {
        Self::new()
    }
}

impl PhysicalReadResults {
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: BTreeSet::new(),
        }
    }

    /// Produces an iterator of planned "read into" chunks.
    ///
    /// This is the planning layer: no I/O is performed here.
    /// Consumers can use this for assertions in tests or custom write targets.
    ///
    /// The iterator yields steps in logical write order and borrows bytes directly
    /// from `self`, avoiding extra allocations in the hot path.
    #[must_use]
    pub fn iter<'a>(&'a self, logical_plan: &'a LogicalReadPlan) -> PhysicalReadResultsIter<'a> {
        PhysicalReadResultsIter {
            entries: &self.entries,
            logical_segments: logical_plan.segments.iter(),
            active_segment: None,
            done: false,
        }
    }

    /// Reads planned data into a writer.
    ///
    /// This is the execution layer on top of [`Self::read_into_iter`].
    /// Each yielded step is written via `seek + write_all`.
    ///
    /// # Errors
    ///
    /// Returns an error if expected physical data is missing or if seeking/writing fails.
    pub fn write<W: Seek + Write>(
        &self,
        logical_plan: &LogicalReadPlan,
        writer: &mut W,
    ) -> eyre::Result<()> {
        for step in self.iter(logical_plan) {
            let step = step?;
            write_step(writer, &step)?;
        }

        Ok(())
    }

    /// Reads the logical plan into a file path (pre-sizing & zero-filling gaps by allocation).
    ///
    /// This is a convenience helper on top of [`Self::read_into_writer`].
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use teamy_mft::read::logical_read_plan::LogicalReadPlan;
    /// # use teamy_mft::read::physical_read_results::PhysicalReadResults;
    /// # fn demo(results: &PhysicalReadResults, plan: &LogicalReadPlan) -> eyre::Result<()> {
    /// results.write_to_path(plan, "mft.bin")?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if creating, seeking, or writing to the output file fails or if expected data is missing.
    pub fn write_to_path(
        &self,
        logical_plan: &LogicalReadPlan,
        output_path: impl AsRef<std::path::Path>,
    ) -> eyre::Result<()> {
        let output_path = output_path.as_ref();
        let _span = info_span!(
            "write_physical_read_results_to_path",
            output_path = %output_path.display(),
            logical_size = logical_plan.total_logical_size().format_human(BINARY),
            logical_segments = logical_plan.segments.len(),
            physical_segments = self.entries.len(),
        )
        .entered();
        debug!("Writing MFT output to {:?}", output_path);

        let file = {
            let _span = info_span!(
                "create_mft_output_file",
                output_path = %output_path.display(),
            )
            .entered();
            std::fs::File::create(output_path)?
        };
        {
            let _span = info_span!(
                "preallocate_mft_output_file",
                output_path = %output_path.display(),
                logical_size_bytes = logical_plan.total_logical_size().get::<byte>(),
            )
            .entered();
            file.set_len(logical_plan.total_logical_size().get::<byte>() as u64)?;
        }

        let mut writer = std::io::BufWriter::new(file);
        {
            let _span = info_span!(
                "write_logical_mft_contents",
                logical_size_bytes = logical_plan.total_logical_size().get::<byte>(),
                logical_segments = logical_plan.segments.len(),
                physical_segments = self.entries.len(),
            )
            .entered();
            self.write(logical_plan, &mut writer)?;
        }

        {
            let _span = info_span!(
                "flush_mft_output_writer",
                output_path = %output_path.display(),
            )
            .entered();
            writer.flush()?;
        }
        Ok(())
    }

    /// Materialize the logical read plan into a contiguous in-memory buffer.
    ///
    /// Sparse gaps remain zero-filled in the returned vector.
    ///
    /// # Errors
    ///
    /// Returns an error if expected physical data is missing.
    pub fn to_vec(&self, logical_plan: &LogicalReadPlan) -> eyre::Result<Vec<u8>> {
        let mut bytes = vec![0u8; logical_plan.total_logical_size().get::<byte>()];
        let mut cursor = Cursor::new(bytes.as_mut_slice());
        self.write(logical_plan, &mut cursor)?;
        Ok(bytes)
    }
}

fn write_step<W: Seek + Write>(
    writer: &mut W,
    step: &PhysicalReadResultsIterValue<'_>,
) -> eyre::Result<()> {
    #[cfg(feature = "tracy")]
    let _span = debug_span!("write_logical_mft_step").entered();

    writer.seek(SeekFrom::Start(step.logical_offset.get::<byte>() as u64))?;
    writer.write_all(step.bytes)?;
    Ok(())
}

impl<'a> Iterator for PhysicalReadResultsIter<'a> {
    type Item = eyre::Result<PhysicalReadResultsIterValue<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }

        loop {
            if let Some(active) = &mut self.active_segment {
                if active.physical_offset_current >= active.physical_offset_end {
                    self.active_segment = None;
                    continue;
                }

                let physical_offset_current = active.physical_offset_current;
                trace!(
                    physical_offset_current = physical_offset_current.get::<byte>(),
                    physical_offset_end = active.physical_offset_end.get::<byte>(),
                    remaining =
                        (active.physical_offset_end - physical_offset_current).format_human(BINARY),
                    "Locating physical data for logical segment",
                );

                let probe = PhysicalReadResultEntry {
                    request: PhysicalReadRequest::new(
                        physical_offset_current,
                        Information::new::<byte>(usize::MAX),
                    ),
                    data: vec![],
                };
                let entry = self.entries.range(..=probe).next_back();
                let Some(entry) = entry else {
                    self.done = true;
                    return Some(Err(eyre::eyre!(
                        "Missing physical read data at offset {physical_offset_current:?} - no entries available"
                    )));
                };
                if !(entry.request.offset <= physical_offset_current
                    && physical_offset_current < entry.request.offset + entry.request.length)
                {
                    self.done = true;
                    return Some(Err(eyre::eyre!(
                        "Missing physical read data at offset {physical_offset_current:?} - not contained in any entry"
                    )));
                }

                let offset_within_entry = physical_offset_current - entry.request.offset;
                let bytes_available = entry.request.length - offset_within_entry;
                let bytes_needed = active.physical_offset_end - physical_offset_current;
                let bytes_to_write = if bytes_available < bytes_needed {
                    bytes_available
                } else {
                    bytes_needed
                };
                let slice = &entry.data[offset_within_entry.get::<byte>()
                    ..(offset_within_entry + bytes_to_write).get::<byte>()];

                let step = PhysicalReadResultsIterValue {
                    logical_offset: active.logical_offset_start
                        + (physical_offset_current - active.physical_offset_start),
                    physical_offset: physical_offset_current,
                    bytes: slice,
                };
                active.physical_offset_current += bytes_to_write;
                return Some(Ok(step));
            }

            let next_logical_segment = self.logical_segments.next()?;
            let Some(physical_segment) = next_logical_segment.as_physical_read_request() else {
                continue;
            };

            trace!(
                ?next_logical_segment,
                "Identifying physical data for logical segment"
            );
            self.active_segment = Some(ActivePhysicalSegment {
                logical_offset_start: next_logical_segment.logical_offset,
                physical_offset_start: physical_segment.offset,
                physical_offset_current: physical_segment.offset,
                physical_offset_end: physical_segment.offset + physical_segment.length,
            });
        }
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
    use crate::read::physical_read_results::PhysicalReadResultsIterValue;
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

        read_results.write_to_path(&read_plan, &path)?;
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
            // i is a usize here; convert explicitly to u8 so the test saturates clearly and
            // avoids clippy's pedantic truncation/sign-loss warnings.
            *b = 64u8 + u8::try_from(i).unwrap(); // distinct content to verify slice is correct
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
        read_results.write_to_path(&read_plan, &path)?;
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.len(), 10);
        for (i, b) in bytes.iter().enumerate() {
            assert_eq!(*b as usize, 100 + i);
        }
        Ok(())
    }

    #[test]
    fn write_plan_steps_can_be_asserted_without_io() -> eyre::Result<()> {
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

        let mut data = vec![0u8; 64];
        for (i, b) in data.iter_mut().enumerate() {
            *b = 64u8 + u8::try_from(i).unwrap();
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

        let plan = read_results
            .iter(&read_plan)
            .collect::<eyre::Result<Vec<PhysicalReadResultsIterValue<'_>>>>()?;

        assert_eq!(plan.len(), 1);
        let step = plan[0];
        assert_eq!(step.logical_offset, Information::new::<byte>(0),);
        assert_eq!(step.physical_offset, Information::new::<byte>(100),);
        assert_eq!(step.length(), Information::new::<byte>(10),);
        assert_eq!(
            step.bytes,
            &[100, 101, 102, 103, 104, 105, 106, 107, 108, 109],
        );
        Ok(())
    }

    #[test]
    fn write_plan_errors_on_missing_physical_data() {
        let read_plan = LogicalReadPlan {
            segments: [LogicalFileSegment {
                logical_offset: Information::new::<byte>(0),
                length: Information::new::<byte>(8),
                kind: LogicalFileSegmentKind::Physical {
                    physical_offset: Information::new::<byte>(100),
                },
            }]
            .into_iter()
            .collect(),
        };

        let read_results = PhysicalReadResults {
            entries: [PhysicalReadResultEntry {
                request: PhysicalReadRequest {
                    offset: Information::new::<byte>(100),
                    length: Information::new::<byte>(4),
                },
                data: vec![1, 2, 3, 4],
            }]
            .into_iter()
            .collect(),
        };

        let err = read_results
            .iter(&read_plan)
            .collect::<eyre::Result<Vec<PhysicalReadResultsIterValue<'_>>>>()
            .expect_err("expected missing data error");
        assert!(err.to_string().contains("Missing physical read data"));
    }
}
