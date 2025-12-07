use crate::read::physical_read_plan::PhysicalReadPlan;
use crate::read::physical_read_request::PhysicalReadRequest;
use std::collections::BTreeSet;
use uom::ConstZero;
use uom::si::usize::Information;

/// A plan for reading a file logically, including sparse segments.
///
/// The MFT is a collection of segments that may be either physical (data present on disk)
/// or sparse (holes, unallocated, zero-filled).
///
/// Each part of the MFT exists somewhere on the physical device.
/// The logical read plan describes how to read the entire MFT.
#[derive(Debug, Clone)]
pub struct LogicalReadPlan {
    pub segments: BTreeSet<LogicalFileSegment>,
}

impl LogicalReadPlan {
    pub fn physical_segments(&self) -> impl Iterator<Item = &LogicalFileSegment> {
        self.segments
            .iter()
            .filter(|s| matches!(s.kind, LogicalFileSegmentKind::Physical { .. }))
    }

    #[must_use]
    pub fn as_physical_read_plan(&self) -> PhysicalReadPlan {
        self.segments
            .iter()
            .filter_map(LogicalFileSegment::as_physical_read_request)
            .collect()
    }

    #[must_use]
    pub fn total_logical_size(&self) -> Information {
        self.segments
            .last()
            .map_or(Information::ZERO, |s| s.logical_offset + s.length)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicalFileSegment {
    pub logical_offset: Information,
    pub length: Information,
    pub kind: LogicalFileSegmentKind,
}
impl LogicalFileSegment {
    #[must_use]
    pub fn as_physical_read_request(&self) -> Option<PhysicalReadRequest> {
        match self.kind {
            LogicalFileSegmentKind::Physical {
                physical_offset: physical_offset_bytes,
            } => Some(PhysicalReadRequest::new(physical_offset_bytes, self.length)),
            LogicalFileSegmentKind::Sparse => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogicalFileSegmentKind {
    Physical { physical_offset: Information },
    Sparse,
}

impl Ord for LogicalFileSegment {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.logical_offset.cmp(&other.logical_offset)
    }
}
impl PartialOrd for LogicalFileSegment {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
