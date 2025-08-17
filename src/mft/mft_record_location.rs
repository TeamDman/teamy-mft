use crate::mft::mft_location::MftLocationOnDisk;
use crate::mft::mft_record_number::MftRecordNumber;
use std::ops::Deref;

#[derive(Debug, Clone, Copy)]
pub struct MftRecordLocationOnDisk(u64);

impl MftRecordLocationOnDisk {
    pub const fn new(offset: u64) -> Self { MftRecordLocationOnDisk(offset) }
    pub fn from_record_number(mft_location: MftLocationOnDisk, record_number: MftRecordNumber) -> Self { mft_location + record_number }
}

impl Deref for MftRecordLocationOnDisk {
    type Target = u64;
    fn deref(&self) -> &Self::Target { &self.0 }
}
