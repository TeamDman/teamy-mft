use eyre::ContextCompat;
use eyre::ensure;
use std::ops::Range;
use uom::ConstZero;
use uom::si::information::byte;
use uom::si::usize::Information;

/// A half-open byte interval with ordered, representable endpoints.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
pub struct ByteRange {
    start: Information,
    end: Information,
}

impl ByteRange {
    /// # Errors
    /// Returns an error if the start is greater than the end.
    pub fn new(start: Information, end: Information) -> eyre::Result<Self> {
        ensure!(start <= end, "Byte range start exceeds its end");
        Ok(Self { start, end })
    }

    /// # Errors
    /// Returns an error if the offset plus length overflows.
    pub fn from_offset_and_length(offset: Information, length: Information) -> eyre::Result<Self> {
        let end = offset
            .get::<byte>()
            .checked_add(length.get::<byte>())
            .wrap_err("Byte range offset plus length overflows")?;
        Ok(Self {
            start: offset,
            end: Information::new::<byte>(end),
        })
    }

    #[must_use]
    pub fn start(&self) -> Information {
        self.start
    }

    #[must_use]
    pub fn end(&self) -> Information {
        self.end
    }

    #[must_use]
    pub fn length(&self) -> Information {
        self.end - self.start
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// Return the non-empty overlap; touching endpoints do not intersect.
    #[must_use]
    pub fn intersection(&self, other: &Self) -> Option<Self> {
        let start = std::cmp::max(self.start, other.start);
        let end = std::cmp::min(self.end, other.end);
        if start >= end {
            None
        } else {
            Some(Self { start, end })
        }
    }

    /// Includes equal ranges and empty ranges at either boundary.
    #[must_use]
    pub fn encompasses(&self, other: &Self) -> bool {
        self.start <= other.start && other.end <= self.end
    }

    /// Take a prefix of at most `chunk_size` bytes and return any remainder.
    /// Zero leaves the range unsplit; empty ranges have no remainder.
    #[must_use]
    pub fn split(&self, chunk_size: Information) -> (Self, Option<Self>) {
        if chunk_size == Information::ZERO || chunk_size >= self.length() {
            return (*self, None);
        }
        // The split point stays below self.end, so this addition cannot overflow.
        let split_point = self.start + chunk_size;
        (
            Self {
                start: self.start,
                end: split_point,
            },
            Some(Self {
                start: split_point,
                end: self.end,
            }),
        )
    }

    /// Rebase a contained interval so the parent's start becomes zero.
    ///
    /// # Errors
    /// Returns an error if this interval is not contained in the parent.
    pub fn relative_to(&self, parent: &Self) -> eyre::Result<Self> {
        ensure!(
            parent.encompasses(self),
            "Byte range is not contained in its parent"
        );
        Ok(Self {
            start: self.start - parent.start,
            end: self.end - parent.start,
        })
    }

    /// # Errors
    /// Returns an error if translating either endpoint overflows.
    pub fn shifted_by(&self, offset: Information) -> eyre::Result<Self> {
        let offset = offset.get::<byte>();
        let start = self
            .start
            .get::<byte>()
            .checked_add(offset)
            .wrap_err("Byte range start overflows when shifted")?;
        let end = self
            .end
            .get::<byte>()
            .checked_add(offset)
            .wrap_err("Byte range end overflows when shifted")?;
        Ok(Self {
            start: Information::new::<byte>(start),
            end: Information::new::<byte>(end),
        })
    }

    /// Expand a non-empty interval to alignment boundaries. Empty intervals stay empty.
    ///
    /// # Errors
    /// Returns an error if alignment is zero or rounding the end up overflows.
    pub fn aligned_to(&self, alignment: Information) -> eyre::Result<Self> {
        let alignment = alignment.get::<byte>();
        ensure!(alignment != 0, "Byte range alignment must be positive");
        if self.is_empty() {
            return Ok(*self);
        }
        let start = self.start.get::<byte>();
        let end = self.end.get::<byte>();
        let remainder = end % alignment;
        let aligned_end = if remainder == 0 {
            end
        } else {
            end.checked_add(alignment - remainder)
                .wrap_err("Byte range end overflows when aligned")?
        };
        Ok(Self {
            start: Information::new::<byte>(start - start % alignment),
            end: Information::new::<byte>(aligned_end),
        })
    }

    #[must_use]
    pub fn byte_indices(&self) -> Range<usize> {
        self.start.get::<byte>()..self.end.get::<byte>()
    }
}

#[cfg(test)]
mod tests {
    use super::ByteRange;
    use uom::si::information::byte;
    use uom::si::usize::Information;

    fn info(bytes: usize) -> Information {
        Information::new::<byte>(bytes)
    }

    #[test]
    fn constructors_preserve_half_open_bounds() -> eyre::Result<()> {
        let range = ByteRange::from_offset_and_length(info(100), info(50))?;
        assert_eq!(range, ByteRange::new(info(100), info(150))?);
        assert_eq!(range.start(), info(100));
        assert_eq!(range.end(), info(150));
        assert_eq!(range.length(), info(50));
        assert_eq!(range.byte_indices(), 100..150);
        assert!(!range.is_empty());
        let empty = ByteRange::from_offset_and_length(info(usize::MAX), info(0))?;
        assert!(empty.is_empty());
        assert_eq!(empty.length(), info(0));
        assert_eq!(empty.byte_indices(), usize::MAX..usize::MAX);
        Ok(())
    }

    #[test]
    fn constructors_reject_reversed_and_overflowing_bounds() -> eyre::Result<()> {
        assert!(ByteRange::new(info(20), info(10)).is_err());
        assert!(ByteRange::from_offset_and_length(info(usize::MAX), info(1)).is_err());
        assert_eq!(
            ByteRange::from_offset_and_length(info(0), info(usize::MAX))?.end(),
            info(usize::MAX)
        );
        Ok(())
    }

    #[test]
    fn intersection_requires_a_shared_byte() -> eyre::Result<()> {
        let range = ByteRange::new(info(10), info(20))?;
        for (start, end, expected) in [
            (5, 15, Some(10..15)),
            (12, 18, Some(12..18)),
            (0, 30, Some(10..20)),
            (0, 10, None),
            (20, 30, None),
            (30, 40, None),
            (12, 12, None),
        ] {
            let other = ByteRange::new(info(start), info(end))?;
            assert_eq!(
                range.intersection(&other).map(|value| value.byte_indices()),
                expected
            );
            assert_eq!(range.intersection(&other), other.intersection(&range));
        }
        Ok(())
    }

    #[test]
    fn rebasing_requires_containment_and_handles_empty_boundaries() -> eyre::Result<()> {
        let parent = ByteRange::new(info(10), info(30))?;
        assert_eq!(
            ByteRange::new(info(15), info(20))?
                .relative_to(&parent)?
                .byte_indices(),
            5..10
        );
        assert_eq!(
            ByteRange::new(info(30), info(30))?
                .relative_to(&parent)?
                .byte_indices(),
            20..20
        );
        for (start, end) in [(9, 20), (20, 31), (31, 31)] {
            assert!(
                ByteRange::new(info(start), info(end))?
                    .relative_to(&parent)
                    .is_err()
            );
        }
        Ok(())
    }

    #[test]
    fn splitting_preserves_bounds_and_finishes_without_empty_remainders() -> eyre::Result<()> {
        for (start, end, chunk_size, expected_chunk, expected_remainder) in [
            (10, 30, 8, 10..18, Some(18..30)),
            (10, 30, 20, 10..30, None),
            (10, 30, usize::MAX, 10..30, None),
            (10, 30, 0, 10..30, None),
            (10, 10, 8, 10..10, None),
            (usize::MAX, usize::MAX, 0, usize::MAX..usize::MAX, None),
            (
                usize::MAX - 10,
                usize::MAX,
                4,
                usize::MAX - 10..usize::MAX - 6,
                Some(usize::MAX - 6..usize::MAX),
            ),
        ] {
            let range = ByteRange::new(info(start), info(end))?;
            let (chunk, remainder) = range.split(info(chunk_size));
            assert_eq!(chunk.byte_indices(), expected_chunk);
            assert_eq!(
                remainder.map(|range| range.byte_indices()),
                expected_remainder
            );
        }
        Ok(())
    }

    #[test]
    fn shifts_preserve_length_and_reject_endpoint_overflow() -> eyre::Result<()> {
        assert_eq!(
            ByteRange::new(info(5), info(10))?
                .shifted_by(info(20))?
                .byte_indices(),
            25..30
        );
        assert!(
            ByteRange::new(info(usize::MAX - 10), info(usize::MAX))?
                .shifted_by(info(1))
                .is_err()
        );
        assert!(
            ByteRange::new(info(usize::MAX), info(usize::MAX))?
                .shifted_by(info(1))
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn alignment_rounds_outward_without_inventing_empty_reads() -> eyre::Result<()> {
        assert_eq!(
            ByteRange::new(info(100), info(150))?
                .aligned_to(info(64))?
                .byte_indices(),
            64..192
        );
        let aligned = ByteRange::new(info(64), info(128))?;
        assert_eq!(aligned.aligned_to(info(64))?, aligned);
        let empty = ByteRange::new(info(13), info(13))?;
        assert_eq!(empty.aligned_to(info(64))?, empty);
        assert!(aligned.aligned_to(info(0)).is_err());
        assert!(
            ByteRange::new(info(usize::MAX - 1), info(usize::MAX))?
                .aligned_to(info(2))
                .is_err()
        );
        Ok(())
    }
}
