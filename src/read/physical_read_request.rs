use uom::ConstZero;
use uom::si::usize::Information;

/// A request to read from a physical device at a specific physical offset.
///
/// Includes the logical offset to place the data in the final output.
///
/// This is used for reading the MFT, which is a file that consists of chunks spread across the disk which we want to reassemble into a single file.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PhysicalReadRequest {
    /// Where on the disk this chunk is located
    pub offset: Information,
    /// How many bytes to physically read
    pub length: Information,
}
impl PhysicalReadRequest {
    #[must_use]
    pub fn new(offset: Information, length: Information) -> Self {
        Self { offset, length }
    }

    pub fn align_to_sector_size(&mut self, sector_size: Information) {
        let past_sector_start = self.offset % sector_size;
        let past_sector_end = (self.offset + self.length) % sector_size;
        if past_sector_start == Information::ZERO && past_sector_end == Information::ZERO {
            return;
        }
        let aligned_start = self.offset - past_sector_start;
        let aligned_end = if past_sector_end == Information::ZERO {
            self.offset + self.length
        } else {
            self.offset + self.length + (sector_size - past_sector_end)
        };
        self.offset = aligned_start;
        self.length = aligned_end - aligned_start;
    }

    #[must_use]
    pub fn physical_end(&self) -> Information {
        self.offset + self.length
    }
}

impl Ord for PhysicalReadRequest {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.offset.cmp(&other.offset) {
            std::cmp::Ordering::Equal => self.length.cmp(&other.length),
            o => o,
        }
    }
}
impl PartialOrd for PhysicalReadRequest {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod test {
    use crate::read::physical_read_request::PhysicalReadRequest;
    use uom::si::information::byte;
    use uom::si::usize::Information;

    #[test]
    fn it_works() {
        let request =
            PhysicalReadRequest::new(Information::new::<byte>(100), Information::new::<byte>(50));
        assert_eq!(request.physical_end(), Information::new::<byte>(150));

        let aligned = {
            let mut r = request;
            r.align_to_sector_size(Information::new::<byte>(64));
            r
        };
        // With 64-byte sectors, [100,150) expands to [64,192)
        assert_eq!(aligned.offset, Information::new::<byte>(64));
        assert_eq!(aligned.length, Information::new::<byte>(128));
        assert_eq!(aligned.physical_end(), Information::new::<byte>(192));
    }

    #[test]
    fn ord_eq_consistency_same_offset_different_length() {
        use std::collections::BTreeSet;
        let mut set = BTreeSet::new();
        let a =
            PhysicalReadRequest::new(Information::new::<byte>(100), Information::new::<byte>(10));
        let b =
            PhysicalReadRequest::new(Information::new::<byte>(100), Information::new::<byte>(20));
        assert!(set.insert(a));
        assert!(set.insert(b)); // both should be kept, as they are not equal
        let v: Vec<_> = set.into_iter().collect();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].length.get::<byte>(), 10);
        assert_eq!(v[1].length.get::<byte>(), 20);
    }

    #[test]
    fn set_deduplicates_identical_requests() {
        use std::collections::BTreeSet;
        let mut set = BTreeSet::new();
        let a =
            PhysicalReadRequest::new(Information::new::<byte>(200), Information::new::<byte>(5));
        let b =
            PhysicalReadRequest::new(Information::new::<byte>(200), Information::new::<byte>(5));
        assert!(set.insert(a));
        assert!(!set.insert(b)); // identical
        assert_eq!(set.len(), 1);
    }
}
