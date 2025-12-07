use crate::mft::mft_location::MftLocationOnDisk;
use crate::mft::mft_record_number::MftRecordNumber;
use std::ops::Deref;
use uom::si::usize::Information;

#[derive(Debug, Clone, Copy)]
pub struct MftRecordLocationOnDisk(Information);

impl MftRecordLocationOnDisk {
    #[must_use]
    pub const fn new(offset: Information) -> Self {
        MftRecordLocationOnDisk(offset)
    }
    #[must_use]
    pub fn from_record_number(
        mft_location: &MftLocationOnDisk,
        record_number: MftRecordNumber,
        record_size: Information,
    ) -> Self {
        mft_location.record_location(record_number, record_size)
    }
}

impl Deref for MftRecordLocationOnDisk {
    type Target = Information;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
