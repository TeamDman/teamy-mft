use std::ops::Deref;
use uom::si::usize::Information;

#[derive(Debug, Clone, Copy)]
pub struct MftRecordLocationOnDisk(Information);

impl MftRecordLocationOnDisk {
    #[must_use]
    pub const fn new(offset: Information) -> Self {
        MftRecordLocationOnDisk(offset)
    }
}

impl Deref for MftRecordLocationOnDisk {
    type Target = Information;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
