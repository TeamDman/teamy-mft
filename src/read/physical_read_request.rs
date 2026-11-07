use crate::read::byte_range::ByteRange;
use uom::si::usize::Information;

/// A request to read a validated byte range from a physical device.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
pub struct PhysicalReadRequest {
    range: ByteRange,
}

impl From<ByteRange> for PhysicalReadRequest {
    fn from(range: ByteRange) -> Self {
        Self { range }
    }
}

impl PhysicalReadRequest {
    #[must_use]
    pub fn range(&self) -> ByteRange {
        self.range
    }

    /// # Errors
    /// Returns an error if alignment is zero or rounding the end up overflows.
    pub fn align_to_sector_size(&mut self, sector_size: Information) -> eyre::Result<()> {
        self.range = self.range.aligned_to(sector_size)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::PhysicalReadRequest;
    use crate::read::byte_range::ByteRange;
    use std::collections::BTreeSet;
    use uom::si::information::byte;
    use uom::si::usize::Information;

    #[test]
    fn sector_alignment_preserves_a_valid_range_on_failure() -> eyre::Result<()> {
        let mut request = PhysicalReadRequest::from(ByteRange::from_offset_and_length(
            Information::new::<byte>(100),
            Information::new::<byte>(50),
        )?);
        request.align_to_sector_size(Information::new::<byte>(64))?;
        assert_eq!(request.range().byte_indices(), 64..192);
        let before = request;
        assert!(
            request
                .align_to_sector_size(Information::new::<byte>(0))
                .is_err()
        );
        assert_eq!(request, before);
        Ok(())
    }

    #[test]
    fn ordered_requests_distinguish_lengths_and_deduplicate_identical_ranges() -> eyre::Result<()> {
        let shorter = PhysicalReadRequest::from(ByteRange::from_offset_and_length(
            Information::new::<byte>(100),
            Information::new::<byte>(10),
        )?);
        let longer = PhysicalReadRequest::from(ByteRange::from_offset_and_length(
            Information::new::<byte>(100),
            Information::new::<byte>(20),
        )?);
        let mut requests = BTreeSet::new();
        assert!(requests.insert(longer));
        assert!(requests.insert(shorter));
        assert!(!requests.insert(shorter));
        assert_eq!(requests.into_iter().collect::<Vec<_>>(), [shorter, longer]);
        Ok(())
    }
}
