use crate::read::byte_range::ByteRange;
use crate::read::logical_read_plan::LogicalReadPlan;
use crate::read::physical_read_request::PhysicalReadRequest;
use std::collections::BTreeSet;
use tracing::instrument;
use uom::ConstZero;
use uom::si::information::byte;
use uom::si::usize::Information;

#[derive(Debug, Clone)]
pub struct PhysicalReadPlan<'plan> {
    logical_plan: &'plan LogicalReadPlan,
    requests: BTreeSet<PhysicalReadRequest>,
}

impl IntoIterator for PhysicalReadPlan<'_> {
    type Item = PhysicalReadRequest;
    type IntoIter = std::collections::btree_set::IntoIter<PhysicalReadRequest>;
    fn into_iter(self) -> Self::IntoIter {
        self.requests.into_iter()
    }
}
impl<'plan> TryFrom<&'plan LogicalReadPlan> for PhysicalReadPlan<'plan> {
    type Error = eyre::Report;

    fn try_from(logical_plan: &'plan LogicalReadPlan) -> Result<Self, Self::Error> {
        let mut requests = BTreeSet::new();
        for segment in &logical_plan.segments {
            let Some(range) = segment.physical_range()? else {
                continue;
            };
            if !range.is_empty() {
                requests.insert(PhysicalReadRequest::from(range));
            }
        }
        Ok(Self {
            logical_plan,
            requests,
        })
    }
}

impl<'plan> PhysicalReadPlan<'plan> {
    #[must_use]
    pub fn logical_plan(&self) -> &'plan LogicalReadPlan {
        self.logical_plan
    }

    /// Merge physically overlapping or contiguous requests. Returns &mut self for chaining.
    ///
    /// # Errors
    ///
    /// Propagates errors constructing the merged physical range.
    pub fn merge_contiguous_reads(&mut self) -> eyre::Result<&mut Self> {
        if self.requests.is_empty() {
            return Ok(self);
        }
        let physical_requests = std::mem::take(&mut self.requests);
        let merged = &mut self.requests;

        for req in physical_requests {
            let Some(last) = merged.pop_last() else {
                merged.insert(req);
                continue;
            };
            if req.range().start() <= last.range().end() {
                merged.insert(PhysicalReadRequest::from(ByteRange::new(
                    last.range().start(),
                    std::cmp::max(last.range().end(), req.range().end()),
                )?));
            } else {
                merged.insert(last);
                merged.insert(req);
            }
        }

        Ok(self)
    }

    /// Split requests into pieces of at most `chunk_size` bytes. Zero leaves them unsplit.
    #[must_use]
    #[instrument(skip_all)]
    pub fn chunked(&self, chunk_size: Information) -> Self {
        let mut out = Self {
            logical_plan: self.logical_plan,
            requests: BTreeSet::new(),
        };
        for req in &self.requests {
            let mut remaining = Some(req.range());
            while let Some(range) = remaining {
                let (chunk, remainder) = range.split(chunk_size);
                out.requests.insert(PhysicalReadRequest::from(chunk));
                remaining = remainder;
            }
        }
        out
    }

    /// Adjust requests so each (offset,length) is 512-byte aligned by expanding outward.
    /// The logical offsets and lengths remain the same; we simply over-read and will trim later.
    ///
    /// # Errors
    ///
    /// Returns an error if alignment would overflow a physical range.
    pub fn align_512(&mut self) -> eyre::Result<&mut Self> {
        if self.requests.is_empty() {
            return Ok(self);
        }
        let sector_size = Information::new::<byte>(512);
        let mut aligned = BTreeSet::new();
        for mut req in self.requests.iter().copied() {
            req.align_to_sector_size(sector_size)?;
            aligned.insert(req);
        }
        self.requests = aligned;
        self.merge_contiguous_reads()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.requests.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.requests.len()
    }

    #[must_use]
    pub fn total_size(&self) -> Information {
        self.requests
            .iter()
            .map(|request| request.range().length())
            .fold(Information::ZERO, |a, b| a + b)
    }
}

#[cfg(test)]
mod test {
    use crate::read::byte_range::ByteRange;
    use crate::read::logical_read_plan::LogicalFileSegment;
    use crate::read::logical_read_plan::LogicalFileSegmentKind;
    use crate::read::logical_read_plan::LogicalReadPlan;
    use crate::read::physical_read_plan::PhysicalReadPlan;
    use uom::si::information::byte;
    use uom::si::usize::Information;

    #[test]
    fn merges_adjacent_physical_segments() -> eyre::Result<()> {
        let logical = LogicalReadPlan {
            segments: [
                LogicalFileSegment {
                    logical_range: ByteRange::from_offset_and_length(
                        Information::new::<byte>(0),
                        Information::new::<byte>(100),
                    )?,
                    kind: LogicalFileSegmentKind::Physical {
                        physical_offset: Information::new::<byte>(0),
                    },
                },
                LogicalFileSegment {
                    logical_range: ByteRange::from_offset_and_length(
                        Information::new::<byte>(100),
                        Information::new::<byte>(50),
                    )?,
                    kind: LogicalFileSegmentKind::Physical {
                        physical_offset: Information::new::<byte>(100),
                    },
                },
            ]
            .into_iter()
            .collect(),
        };
        let mut physical = PhysicalReadPlan::try_from(&logical)?;
        physical.merge_contiguous_reads()?;
        assert!(std::ptr::eq(physical.logical_plan(), &logical));
        assert_eq!(physical.len(), 1, "Expected contiguous reads to merge");
        let reqs: Vec<_> = physical.clone().into_iter().collect();
        assert_eq!(reqs[0].range().start().get::<byte>(), 0usize);
        assert_eq!(reqs[0].range().length().get::<byte>(), 150usize);
        assert_eq!(physical.total_size().get::<byte>(), 150usize);
        Ok(())
    }

    #[test]
    fn non_adjacent_does_not_merge() -> eyre::Result<()> {
        let logical = LogicalReadPlan {
            segments: [
                LogicalFileSegment {
                    logical_range: ByteRange::from_offset_and_length(
                        Information::new::<byte>(0),
                        Information::new::<byte>(100),
                    )?,
                    kind: LogicalFileSegmentKind::Physical {
                        physical_offset: Information::new::<byte>(0),
                    },
                },
                LogicalFileSegment {
                    logical_range: ByteRange::from_offset_and_length(
                        Information::new::<byte>(100),
                        Information::new::<byte>(50),
                    )?,
                    kind: LogicalFileSegmentKind::Physical {
                        physical_offset: Information::new::<byte>(101),
                    },
                },
            ]
            .into_iter()
            .collect(),
        };
        let mut physical = PhysicalReadPlan::try_from(&logical)?;
        physical.merge_contiguous_reads()?;
        assert_eq!(physical.len(), 2, "Non-contiguous reads must not merge");
        Ok(())
    }

    #[test]
    fn regression_merging_unions_overlapping_contained_and_touching_requests() -> eyre::Result<()> {
        let logical = LogicalReadPlan {
            segments: [(0, 1024), (512, 256), (768, 1024), (1792, 256)]
                .into_iter()
                .enumerate()
                .map(|(index, (offset, length))| {
                    Ok(LogicalFileSegment {
                        logical_range: ByteRange::from_offset_and_length(
                            Information::new::<byte>(index * 2048),
                            Information::new::<byte>(length),
                        )?,
                        kind: LogicalFileSegmentKind::Physical {
                            physical_offset: Information::new::<byte>(offset),
                        },
                    })
                })
                .collect::<eyre::Result<_>>()?,
        };
        let mut physical = PhysicalReadPlan::try_from(&logical)?;
        physical.merge_contiguous_reads()?;
        let requests = physical.into_iter().collect::<Vec<_>>();

        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].range().start().get::<byte>(), 0);
        assert_eq!(requests[0].range().length().get::<byte>(), 2048);
        Ok(())
    }

    #[test]
    fn chunking_splits_without_merging_chunks() -> eyre::Result<()> {
        let logical = LogicalReadPlan {
            segments: [LogicalFileSegment {
                logical_range: ByteRange::from_offset_and_length(
                    Information::new::<byte>(0),
                    Information::new::<byte>(300),
                )?,
                kind: LogicalFileSegmentKind::Physical {
                    physical_offset: Information::new::<byte>(0),
                },
            }]
            .into_iter()
            .collect(),
        };
        let physical = PhysicalReadPlan::try_from(&logical)?;
        for chunk_size in [0, 300, usize::MAX] {
            let unsplit = physical.chunked(Information::new::<byte>(chunk_size));
            assert!(std::ptr::eq(unsplit.logical_plan(), &logical));
            assert_eq!(
                unsplit.into_iter().collect::<Vec<_>>(),
                physical.clone().into_iter().collect::<Vec<_>>()
            );
        }
        let chunked = physical.chunked(Information::new::<byte>(128));
        assert!(std::ptr::eq(chunked.logical_plan(), &logical));
        // 300 bytes in 128-byte chunks => 128,128,44 (3 chunks)
        assert_eq!(chunked.len(), 3usize, "Chunking should split into 3 parts");
        let reqs: Vec<_> = chunked.clone().into_iter().collect();
        assert_eq!(reqs[0].range().start().get::<byte>(), 0usize);
        assert_eq!(reqs[0].range().length().get::<byte>(), 128usize);
        assert_eq!(reqs[1].range().start().get::<byte>(), 128usize);
        assert_eq!(reqs[1].range().length().get::<byte>(), 128usize);
        assert_eq!(reqs[2].range().start().get::<byte>(), 256usize);
        assert_eq!(reqs[2].range().length().get::<byte>(), 44usize);
        assert_eq!(
            chunked.total_size().get::<byte>(),
            300usize,
            "Total requested should remain constant"
        );
        Ok(())
    }

    #[test]
    fn chunking_respects_exact_division() -> eyre::Result<()> {
        let logical = LogicalReadPlan {
            segments: [LogicalFileSegment {
                logical_range: ByteRange::from_offset_and_length(
                    Information::new::<byte>(0),
                    Information::new::<byte>(4096),
                )?,
                kind: LogicalFileSegmentKind::Physical {
                    physical_offset: Information::new::<byte>(4096),
                },
            }]
            .into_iter()
            .collect(),
        };
        let physical = PhysicalReadPlan::try_from(&logical)?;
        let c = physical.chunked(Information::new::<byte>(1024));
        assert_eq!(c.len(), 4usize);
        let reqs: Vec<_> = c.clone().into_iter().collect();
        for (i, req) in reqs.iter().enumerate() {
            assert_eq!(req.range().start().get::<byte>(), 4096usize + i * 1024usize);
            assert_eq!(req.range().length().get::<byte>(), 1024usize);
        }
        Ok(())
    }

    #[test]
    fn sparse_and_zero_length_segments_require_no_physical_reads() -> eyre::Result<()> {
        let logical = LogicalReadPlan {
            segments: [
                LogicalFileSegment {
                    logical_range: ByteRange::from_offset_and_length(
                        Information::new::<byte>(0),
                        Information::new::<byte>(0),
                    )?,
                    kind: LogicalFileSegmentKind::Physical {
                        physical_offset: Information::new::<byte>(0),
                    },
                },
                LogicalFileSegment {
                    logical_range: ByteRange::from_offset_and_length(
                        Information::new::<byte>(128),
                        Information::new::<byte>(512),
                    )?,
                    kind: LogicalFileSegmentKind::Sparse,
                },
            ]
            .into_iter()
            .collect(),
        };
        let physical = PhysicalReadPlan::try_from(&logical)?;
        assert!(physical.is_empty());
        assert_eq!(physical.total_size().get::<byte>(), 0);
        assert!(std::ptr::eq(physical.logical_plan(), &logical));
        Ok(())
    }

    #[test]
    fn alignment_and_chunking_preserve_the_logical_plan() -> eyre::Result<()> {
        let logical = LogicalReadPlan {
            segments: [
                LogicalFileSegment {
                    logical_range: ByteRange::from_offset_and_length(
                        Information::new::<byte>(0),
                        Information::new::<byte>(300),
                    )?,
                    kind: LogicalFileSegmentKind::Physical {
                        physical_offset: Information::new::<byte>(13),
                    },
                },
                LogicalFileSegment {
                    logical_range: ByteRange::from_offset_and_length(
                        Information::new::<byte>(300),
                        Information::new::<byte>(700),
                    )?,
                    kind: LogicalFileSegmentKind::Physical {
                        physical_offset: Information::new::<byte>(600),
                    },
                },
            ]
            .into_iter()
            .collect(),
        };
        let mut physical = PhysicalReadPlan::try_from(&logical)?;
        physical.align_512()?;
        assert_eq!(physical.len(), 1);
        assert_eq!(physical.total_size().get::<byte>(), 1536);
        assert!(std::ptr::eq(physical.logical_plan(), &logical));
        let chunked = physical.chunked(Information::new::<byte>(512));
        assert!(std::ptr::eq(chunked.logical_plan(), &logical));
        assert_eq!(chunked.len(), 3);
        for (index, request) in chunked.into_iter().enumerate() {
            assert_eq!(request.range().start().get::<byte>(), index * 512);
            assert_eq!(request.range().length().get::<byte>(), 512);
        }
        Ok(())
    }

    #[test]
    fn overflowing_physical_range_is_rejected_during_planning() -> eyre::Result<()> {
        let logical = LogicalReadPlan {
            segments: [LogicalFileSegment {
                logical_range: ByteRange::from_offset_and_length(
                    Information::new::<byte>(0),
                    Information::new::<byte>(1),
                )?,
                kind: LogicalFileSegmentKind::Physical {
                    physical_offset: Information::new::<byte>(usize::MAX),
                },
            }]
            .into_iter()
            .collect(),
        };
        assert!(PhysicalReadPlan::try_from(&logical).is_err());
        Ok(())
    }

    #[test]
    fn alignment_overflow_preserves_the_existing_requests() -> eyre::Result<()> {
        let logical = LogicalReadPlan {
            segments: [LogicalFileSegment {
                logical_range: ByteRange::from_offset_and_length(
                    Information::new::<byte>(0),
                    Information::new::<byte>(1),
                )?,
                kind: LogicalFileSegmentKind::Physical {
                    physical_offset: Information::new::<byte>(usize::MAX - 1),
                },
            }]
            .into_iter()
            .collect(),
        };
        let mut physical = PhysicalReadPlan::try_from(&logical)?;
        let requests = physical.clone().into_iter().collect::<Vec<_>>();
        assert!(physical.align_512().is_err());
        assert_eq!(physical.into_iter().collect::<Vec<_>>(), requests);
        Ok(())
    }
}
