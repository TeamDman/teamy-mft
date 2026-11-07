use crate::read::byte_range::ByteRange;
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
    #[must_use]
    pub fn total_logical_size(&self) -> Information {
        self.segments
            .iter()
            .map(|segment| segment.logical_range.end())
            .max()
            .unwrap_or(Information::ZERO)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct LogicalFileSegment {
    pub logical_range: ByteRange,
    pub kind: LogicalFileSegmentKind,
}
impl LogicalFileSegment {
    /// Returns the allocated physical range, or `None` for a sparse segment.
    ///
    /// # Errors
    ///
    /// Returns an error if the physical offset and length overflow.
    pub fn physical_range(&self) -> eyre::Result<Option<ByteRange>> {
        match self.kind {
            LogicalFileSegmentKind::Physical { physical_offset } => {
                ByteRange::from_offset_and_length(physical_offset, self.logical_range.length())
                    .map(Some)
            }
            LogicalFileSegmentKind::Sparse => Ok(None),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogicalFileSegmentKind {
    Physical { physical_offset: Information },
    Sparse,
}

#[cfg(test)]
mod tests {
    use super::LogicalFileSegment;
    use super::LogicalFileSegmentKind;
    use super::LogicalReadPlan;
    use crate::read::byte_range::ByteRange;
    use std::collections::BTreeSet;
    use uom::ConstZero;
    use uom::si::information::byte;
    use uom::si::usize::Information;

    #[test]
    fn same_start_distinct_segments_are_retained() -> eyre::Result<()> {
        let range = ByteRange::new(Information::new::<byte>(0), Information::new::<byte>(64))?;
        let plan = LogicalReadPlan {
            segments: [
                LogicalFileSegment {
                    logical_range: range,
                    kind: LogicalFileSegmentKind::Physical {
                        physical_offset: Information::new::<byte>(100),
                    },
                },
                LogicalFileSegment {
                    logical_range: range,
                    kind: LogicalFileSegmentKind::Physical {
                        physical_offset: Information::new::<byte>(200),
                    },
                },
                LogicalFileSegment {
                    logical_range: range,
                    kind: LogicalFileSegmentKind::Sparse,
                },
                LogicalFileSegment {
                    logical_range: ByteRange::new(
                        Information::new::<byte>(0),
                        Information::new::<byte>(128),
                    )?,
                    kind: LogicalFileSegmentKind::Sparse,
                },
            ]
            .into_iter()
            .collect(),
        };

        assert_eq!(plan.segments.len(), 4);
        Ok(())
    }

    #[test]
    fn total_logical_size_uses_the_largest_end() -> eyre::Result<()> {
        let plan = LogicalReadPlan {
            segments: [
                LogicalFileSegment {
                    logical_range: ByteRange::new(
                        Information::new::<byte>(0),
                        Information::new::<byte>(100),
                    )?,
                    kind: LogicalFileSegmentKind::Sparse,
                },
                LogicalFileSegment {
                    logical_range: ByteRange::new(
                        Information::new::<byte>(50),
                        Information::new::<byte>(60),
                    )?,
                    kind: LogicalFileSegmentKind::Physical {
                        physical_offset: Information::new::<byte>(200),
                    },
                },
            ]
            .into_iter()
            .collect(),
        };

        assert_eq!(plan.total_logical_size(), Information::new::<byte>(100));
        Ok(())
    }

    #[test]
    fn empty_plans_and_ranges_preserve_their_endpoints() -> eyre::Result<()> {
        let mut plan = LogicalReadPlan {
            segments: BTreeSet::new(),
        };
        assert_eq!(plan.total_logical_size(), Information::ZERO);

        let endpoint = Information::new::<byte>(usize::MAX);
        let segment = LogicalFileSegment {
            logical_range: ByteRange::new(endpoint, endpoint)?,
            kind: LogicalFileSegmentKind::Physical {
                physical_offset: endpoint,
            },
        };
        assert!(segment.physical_range()?.unwrap().is_empty());
        plan.segments.insert(segment);
        assert_eq!(plan.total_logical_size(), endpoint);
        Ok(())
    }

    #[test]
    fn physical_range_uses_the_logical_range_length() -> eyre::Result<()> {
        let logical_range =
            ByteRange::new(Information::new::<byte>(12), Information::new::<byte>(40))?;
        let segment = LogicalFileSegment {
            logical_range,
            kind: LogicalFileSegmentKind::Physical {
                physical_offset: Information::new::<byte>(100),
            },
        };
        assert_eq!(
            segment.physical_range()?,
            Some(ByteRange::new(
                Information::new::<byte>(100),
                Information::new::<byte>(128),
            )?)
        );
        let sparse = LogicalFileSegment {
            logical_range,
            kind: LogicalFileSegmentKind::Sparse,
        };
        assert_eq!(sparse.physical_range()?, None);
        Ok(())
    }
}
