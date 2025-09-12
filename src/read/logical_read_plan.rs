use uom::si::information::byte;
use uom::si::u64::Information;

use crate::read::physical_read_plan::PhysicalReadPlan;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogicalReadSegmentKind {
    Physical { physical_offset_bytes: u64 },
    Sparse,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicalReadSegment {
    pub logical_offset_bytes: u64,
    pub length_bytes: u64,
    pub kind: LogicalReadSegmentKind,
}

#[derive(Debug, Clone)]
pub struct LogicalReadPlan {
    pub segments: Vec<LogicalReadSegment>,
    pub total_logical_size_bytes: u64,
}

impl LogicalReadPlan {
    pub fn physical_segments(&self) -> impl Iterator<Item = &LogicalReadSegment> {
        self.segments
            .iter()
            .filter(|s| matches!(s.kind, LogicalReadSegmentKind::Physical { .. }))
    }

    /// Convert a logical plan into a physical read plan ignoring sparse segments (no chunking/merging).
    pub fn into_physical_plan(&self) -> PhysicalReadPlan {
        let mut physical_plan = PhysicalReadPlan::new();
        for seg in &self.segments {
            if let LogicalReadSegmentKind::Physical {
                physical_offset_bytes,
            } = seg.kind
            {
                physical_plan.push(
                    Information::new::<byte>(physical_offset_bytes),
                    Information::new::<byte>(seg.logical_offset_bytes),
                    Information::new::<byte>(seg.length_bytes),
                );
            }
        }
        physical_plan
    }
}