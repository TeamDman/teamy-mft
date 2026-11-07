use crate::read::logical_read_plan::LogicalFileSegment;
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
impl<'plan> From<&'plan LogicalReadPlan> for PhysicalReadPlan<'plan> {
    fn from(logical_plan: &'plan LogicalReadPlan) -> Self {
        Self {
            logical_plan,
            requests: logical_plan
                .segments
                .iter()
                .filter_map(LogicalFileSegment::as_physical_read_request)
                .filter(|request| request.length != Information::ZERO)
                .collect(),
        }
    }
}

impl<'plan> PhysicalReadPlan<'plan> {
    #[must_use]
    pub fn logical_plan(&self) -> &'plan LogicalReadPlan {
        self.logical_plan
    }

    /// Merge overlapping or contiguous physical requests. Returns &mut self for chaining.
    pub fn merge_contiguous_reads(&mut self) -> &mut Self {
        if self.requests.is_empty() {
            return self;
        }
        let physical_requests = std::mem::take(&mut self.requests);
        let merged = &mut self.requests;

        // Sorted offsets let each request extend the current physical interval's union.
        for req in physical_requests {
            let Some(mut last) = merged.pop_last() else {
                merged.insert(req);
                continue;
            };
            if req.offset <= last.physical_end() {
                let end = std::cmp::max(last.physical_end(), req.physical_end());
                last.length = end - last.offset;
                merged.insert(last);
            } else {
                // No merge; re-insert last and insert new
                merged.insert(last);
                merged.insert(req);
            }
        }

        self
    }

    /// Split requests into uniform <= `chunk_size` pieces. Returns a new plan.
    #[must_use]
    #[instrument(skip_all)]
    pub fn chunked(&self, chunk_size: Information) -> Self {
        if chunk_size == Information::ZERO {
            return self.clone();
        }
        let mut out = Self {
            logical_plan: self.logical_plan,
            requests: BTreeSet::new(),
        };
        for req in &self.requests {
            let mut remaining = req.length;
            let mut current_physical_offset = req.offset;
            while remaining > Information::ZERO {
                let segment_size = if remaining > chunk_size {
                    chunk_size
                } else {
                    remaining
                };
                out.requests.insert(PhysicalReadRequest::new(
                    current_physical_offset,
                    segment_size,
                ));
                current_physical_offset += segment_size;
                remaining -= segment_size;
            }
        }
        out
    }

    /// Adjust requests so each (offset,length) is 512-byte aligned by expanding outward.
    /// The logical offsets and lengths remain the same; we simply over-read and will trim later.
    pub fn align_512(&mut self) -> &mut Self {
        if self.requests.is_empty() {
            return self;
        }
        let sector_size = Information::new::<byte>(512);
        for mut req in std::mem::take(&mut self.requests) {
            req.align_to_sector_size(sector_size);
            self.requests.insert(req);
        }
        self.merge_contiguous_reads();
        self
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
            .map(|r| r.length)
            .fold(Information::ZERO, |a, b| a + b)
    }
}

#[cfg(test)]
mod test {
    use crate::read::logical_read_plan::LogicalFileSegment;
    use crate::read::logical_read_plan::LogicalFileSegmentKind;
    use crate::read::logical_read_plan::LogicalReadPlan;
    use crate::read::physical_read_plan::PhysicalReadPlan;
    use uom::si::information::byte;
    use uom::si::usize::Information;

    #[test]
    fn merges_adjacent_physical_segments() {
        let logical = LogicalReadPlan {
            segments: [
                LogicalFileSegment {
                    logical_offset: Information::new::<byte>(0),
                    length: Information::new::<byte>(100),
                    kind: LogicalFileSegmentKind::Physical {
                        physical_offset: Information::new::<byte>(0),
                    },
                },
                LogicalFileSegment {
                    logical_offset: Information::new::<byte>(100),
                    length: Information::new::<byte>(50),
                    kind: LogicalFileSegmentKind::Physical {
                        physical_offset: Information::new::<byte>(100),
                    },
                },
            ]
            .into_iter()
            .collect(),
        };
        let mut physical = PhysicalReadPlan::from(&logical);
        physical.merge_contiguous_reads();
        assert!(std::ptr::eq(physical.logical_plan(), &logical));
        assert_eq!(physical.len(), 1, "Expected contiguous reads to merge");
        let reqs: Vec<_> = physical.clone().into_iter().collect();
        assert_eq!(reqs[0].offset.get::<byte>(), 0usize);
        assert_eq!(reqs[0].length.get::<byte>(), 150usize);
        assert_eq!(physical.total_size().get::<byte>(), 150usize);
    }

    #[test]
    fn non_adjacent_does_not_merge() {
        let logical = LogicalReadPlan {
            segments: [
                LogicalFileSegment {
                    logical_offset: Information::new::<byte>(0),
                    length: Information::new::<byte>(100),
                    kind: LogicalFileSegmentKind::Physical {
                        physical_offset: Information::new::<byte>(0),
                    },
                },
                LogicalFileSegment {
                    logical_offset: Information::new::<byte>(100),
                    length: Information::new::<byte>(50),
                    kind: LogicalFileSegmentKind::Physical {
                        physical_offset: Information::new::<byte>(101),
                    },
                },
            ]
            .into_iter()
            .collect(),
        };
        let mut physical = PhysicalReadPlan::from(&logical);
        physical.merge_contiguous_reads();
        assert_eq!(physical.len(), 2, "Non-contiguous reads must not merge");
    }

    #[test]
    fn regression_merging_unions_overlapping_contained_and_touching_requests() {
        let logical = LogicalReadPlan {
            segments: [(0, 1024), (512, 256), (768, 1024), (1792, 256)]
                .into_iter()
                .enumerate()
                .map(|(index, (offset, length))| LogicalFileSegment {
                    logical_offset: Information::new::<byte>(index * 2048),
                    length: Information::new::<byte>(length),
                    kind: LogicalFileSegmentKind::Physical {
                        physical_offset: Information::new::<byte>(offset),
                    },
                })
                .collect(),
        };
        let mut physical = PhysicalReadPlan::from(&logical);
        physical.merge_contiguous_reads();
        let requests = physical.into_iter().collect::<Vec<_>>();

        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].offset.get::<byte>(), 0);
        assert_eq!(requests[0].length.get::<byte>(), 2048);
    }

    #[test]
    fn chunking_splits_without_merging_chunks() {
        let logical = LogicalReadPlan {
            segments: [LogicalFileSegment {
                logical_offset: Information::new::<byte>(0),
                length: Information::new::<byte>(300),
                kind: LogicalFileSegmentKind::Physical {
                    physical_offset: Information::new::<byte>(0),
                },
            }]
            .into_iter()
            .collect(),
        };
        let physical = PhysicalReadPlan::from(&logical);
        let chunked = physical.chunked(Information::new::<byte>(128));
        assert!(std::ptr::eq(chunked.logical_plan(), &logical));
        // 300 bytes in 128-byte chunks => 128,128,44 (3 chunks)
        assert_eq!(chunked.len(), 3usize, "Chunking should split into 3 parts");
        let reqs: Vec<_> = chunked.clone().into_iter().collect();
        assert_eq!(reqs[0].offset.get::<byte>(), 0usize);
        assert_eq!(reqs[0].length.get::<byte>(), 128usize);
        assert_eq!(reqs[1].offset.get::<byte>(), 128usize);
        assert_eq!(reqs[1].length.get::<byte>(), 128usize);
        assert_eq!(reqs[2].offset.get::<byte>(), 256usize);
        assert_eq!(reqs[2].length.get::<byte>(), 44usize);
        assert_eq!(
            chunked.total_size().get::<byte>(),
            300usize,
            "Total requested should remain constant"
        );
    }

    #[test]
    fn chunking_respects_exact_division() {
        let logical = LogicalReadPlan {
            segments: [LogicalFileSegment {
                logical_offset: Information::new::<byte>(0),
                length: Information::new::<byte>(4096),
                kind: LogicalFileSegmentKind::Physical {
                    physical_offset: Information::new::<byte>(4096),
                },
            }]
            .into_iter()
            .collect(),
        };
        let physical = PhysicalReadPlan::from(&logical);
        let c = physical.chunked(Information::new::<byte>(1024));
        assert_eq!(c.len(), 4usize);
        let reqs: Vec<_> = c.clone().into_iter().collect();
        for (i, req) in reqs.iter().enumerate() {
            assert_eq!(req.offset.get::<byte>(), 4096usize + i * 1024usize);
            assert_eq!(req.length.get::<byte>(), 1024usize);
        }
    }

    #[test]
    fn sparse_and_zero_length_segments_require_no_physical_reads() {
        let logical = LogicalReadPlan {
            segments: [
                LogicalFileSegment {
                    logical_offset: Information::new::<byte>(0),
                    length: Information::new::<byte>(0),
                    kind: LogicalFileSegmentKind::Physical {
                        physical_offset: Information::new::<byte>(0),
                    },
                },
                LogicalFileSegment {
                    logical_offset: Information::new::<byte>(128),
                    length: Information::new::<byte>(512),
                    kind: LogicalFileSegmentKind::Sparse,
                },
            ]
            .into_iter()
            .collect(),
        };
        let physical = PhysicalReadPlan::from(&logical);
        assert!(physical.is_empty());
        assert_eq!(physical.total_size().get::<byte>(), 0);
        assert!(std::ptr::eq(physical.logical_plan(), &logical));
    }

    #[test]
    fn alignment_and_chunking_preserve_the_logical_plan() {
        let logical = LogicalReadPlan {
            segments: [
                LogicalFileSegment {
                    logical_offset: Information::new::<byte>(0),
                    length: Information::new::<byte>(300),
                    kind: LogicalFileSegmentKind::Physical {
                        physical_offset: Information::new::<byte>(13),
                    },
                },
                LogicalFileSegment {
                    logical_offset: Information::new::<byte>(300),
                    length: Information::new::<byte>(700),
                    kind: LogicalFileSegmentKind::Physical {
                        physical_offset: Information::new::<byte>(600),
                    },
                },
            ]
            .into_iter()
            .collect(),
        };
        let mut physical = PhysicalReadPlan::from(&logical);
        physical.align_512();
        assert_eq!(physical.len(), 1);
        assert_eq!(physical.total_size().get::<byte>(), 1536);
        assert!(std::ptr::eq(physical.logical_plan(), &logical));
        let chunked = physical.chunked(Information::new::<byte>(512));
        assert!(std::ptr::eq(chunked.logical_plan(), &logical));
        assert_eq!(chunked.len(), 3);
        for (index, request) in chunked.into_iter().enumerate() {
            assert_eq!(request.offset.get::<byte>(), index * 512);
            assert_eq!(request.length.get::<byte>(), 512);
        }
    }
}
