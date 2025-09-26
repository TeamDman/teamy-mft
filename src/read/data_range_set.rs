use std::ops::RangeInclusive;

use range_set::RangeSet;

pub struct DataRangeSet {
    inner: RangeSet<[RangeInclusive<usize>; 16]>,
}
impl DataRangeSet {
    pub fn new() -> Self {
        Self {
            inner: RangeSet::new(),
        }
    }

    pub fn insert_range(&mut self, range: RangeInclusive<usize>) {
        self.inner.insert_range(range);
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}