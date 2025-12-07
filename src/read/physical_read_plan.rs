use crate::read::physical_read_request::PhysicalReadRequest;
use crate::read::physical_read_results::PhysicalReadResults;
use crate::read::physical_reader::PhysicalReader;
use std::collections::BTreeSet;
use uom::ConstZero;
use uom::si::information::byte;
use uom::si::usize::Information;
use windows::core::PCWSTR;
use windows::core::Param;

#[derive(Debug, Default, Clone)]
pub struct PhysicalReadPlan {
    requests: BTreeSet<PhysicalReadRequest>,
    zero_length_behavior: ZeroLengthPushBehaviour,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum ZeroLengthPushBehaviour {
    #[default]
    Panic,
    NoOp,
}

const MAX_IN_FLIGHT_IO: usize = 32;
impl IntoIterator for PhysicalReadPlan {
    type Item = PhysicalReadRequest;
    type IntoIter = std::collections::btree_set::IntoIter<PhysicalReadRequest>;
    fn into_iter(self) -> Self::IntoIter {
        self.requests.into_iter()
    }
}
impl FromIterator<PhysicalReadRequest> for PhysicalReadPlan {
    fn from_iter<T: IntoIterator<Item = PhysicalReadRequest>>(iter: T) -> Self {
        let mut plan = PhysicalReadPlan::new();
        for req in iter {
            plan.push(req);
        }
        plan
    }
}

impl PhysicalReadPlan {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_zero_length_behavior(&mut self, behavior: ZeroLengthPushBehaviour) -> &mut Self {
        self.zero_length_behavior = behavior;
        self
    }

    /// # Panics
    ///
    /// Panics if the request length is zero and the zero-length behavior is `Panic`.
    pub fn push(&mut self, request: PhysicalReadRequest) -> &mut Self {
        if request.length == Information::ZERO {
            match self.zero_length_behavior {
                ZeroLengthPushBehaviour::Panic => panic!("Zero-length push detected"),
                ZeroLengthPushBehaviour::NoOp => return self,
            }
        }
        self.requests.insert(request);
        self
    }

    /// Merge physically contiguous requests. Returns &mut self for chaining.
    pub fn merge_contiguous_reads(&mut self) -> &mut Self {
        if self.requests.is_empty() {
            return self;
        }
        let physical_requests = std::mem::take(&mut self.requests);
        let merged = &mut self.requests;

        // The BTreeSet is sorted by (physical_offset, length), so we can just iterate and merge adjacent ones.
        for req in physical_requests {
            let Some(mut last) = merged.pop_last() else {
                merged.insert(req);
                continue;
            };
            if last.physical_end() == req.offset {
                // This begins where the previous one ends, merge together
                last.length += req.length;
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
    pub fn chunked(&self, chunk_size: Information) -> Self {
        if chunk_size == Information::ZERO {
            return self.clone();
        }
        let mut out = PhysicalReadPlan::new();
        for req in &self.requests {
            let mut remaining = req.length;
            let mut current_physical_offset = req.offset;
            while remaining > Information::ZERO {
                let segment_size = if remaining > chunk_size {
                    chunk_size
                } else {
                    remaining
                };
                out.push(PhysicalReadRequest::new(
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
            self.push(req);
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

    /// Read the requested ranges from the given file handle.
    ///
    /// # Errors
    ///
    /// Returns an error if opening the file, enqueuing IO operations, or reading fails.
    pub fn read(self, filename: impl Param<PCWSTR>) -> eyre::Result<PhysicalReadResults> {
        if self.is_empty() {
            return Ok(PhysicalReadResults::new());
        }
        PhysicalReader::try_new(filename, self.requests, MAX_IN_FLIGHT_IO)?.read_all()
    }
}

#[cfg(test)]
mod test {
    use crate::read::physical_read_plan::PhysicalReadPlan;
    use crate::read::physical_read_plan::ZeroLengthPushBehaviour;
    use crate::read::physical_read_request::PhysicalReadRequest;
    use uom::si::information::byte;
    use uom::si::usize::Information;

    fn info(bytes: impl Into<usize>) -> Information {
        Information::new::<byte>(bytes.into())
    }

    #[test]
    fn merge_adjacent_pushes() {
        let mut r = PhysicalReadPlan::new();
        r.push(PhysicalReadRequest::new(info(0usize), info(100usize)));
        r.push(PhysicalReadRequest::new(info(100usize), info(50usize))); // contiguous -> should merge after merge_contiguous_reads
        r.merge_contiguous_reads();
        assert_eq!(r.len(), 1usize, "Expected contiguous pushes to merge");
        let reqs: Vec<_> = r.clone().into_iter().collect();
        assert_eq!(reqs[0].offset.get::<byte>(), 0usize);
        assert_eq!(reqs[0].length.get::<byte>(), 150usize);
        assert_eq!(r.total_size().get::<byte>(), 150usize);
    }

    #[test]
    fn non_adjacent_does_not_merge() {
        let mut r = PhysicalReadPlan::new();
        r.push(PhysicalReadRequest::new(info(0usize), info(100usize)));
        r.push(PhysicalReadRequest::new(info(101usize), info(50usize))); // gap of 1
        r.merge_contiguous_reads();
        assert_eq!(r.len(), 2usize, "Non-contiguous pushes must not merge");
    }

    #[test]
    fn chunking_splits_without_merging_chunks() {
        let mut r = PhysicalReadPlan::new();
        r.push(PhysicalReadRequest::new(info(0usize), info(300usize))); // single extent
        let chunked = r.chunked(info(128usize));
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
        let mut r = PhysicalReadPlan::new();
        r.push(PhysicalReadRequest::new(info(4096usize), info(4096usize)));
        let c = r.chunked(info(1024usize));
        assert_eq!(c.len(), 4usize);
        let reqs: Vec<_> = c.clone().into_iter().collect();
        for (i, req) in reqs.iter().enumerate() {
            assert_eq!(req.offset.get::<byte>(), 4096usize + i * 1024usize);
            assert_eq!(req.length.get::<byte>(), 1024usize);
        }
    }

    #[test]
    fn zero_length_push_ignored() {
        let mut r = PhysicalReadPlan::new();
        r.set_zero_length_behavior(ZeroLengthPushBehaviour::NoOp);
        r.push(PhysicalReadRequest::new(info(0usize), info(0usize)));
        assert!(r.is_empty());
    }

    #[test]
    #[should_panic(expected = "Zero-length push detected")]
    fn zero_length_push_panics_by_default() {
        let mut r = PhysicalReadPlan::new();
        // default is Panic
        r.push(PhysicalReadRequest::new(info(0usize), info(0usize)));
    }
}
