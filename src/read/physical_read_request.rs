use uom::si::u64::Information;
// ---------------- Physical Read Plan ----------------
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PhysicalReadRequest {
    pub physical_offset: Information,
    pub logical_offset: Information,
    pub length: Information,
}
impl PhysicalReadRequest {
    pub fn new(
        physical_offset: Information,
        logical_offset: Information,
        length: Information,
    ) -> Self {
        Self {
            physical_offset,
            logical_offset,
            length,
        }
    }
    pub fn physical_end(&self) -> Information {
        self.physical_offset + self.length
    }
    pub fn logical_end(&self) -> Information {
        self.logical_offset + self.length
    }
}
impl PartialOrd for PhysicalReadRequest {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for PhysicalReadRequest {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.physical_offset.cmp(&other.physical_offset)
    }
}