use std::ops::Deref;

#[derive(Debug, Clone, Copy)]
pub struct MftRecordNumber {
    pub record_number: u64,
}
impl MftRecordNumber {
    pub const MFT_ROOT: MftRecordNumber = MftRecordNumber { record_number: 0 };
}
impl From<u64> for MftRecordNumber {
    fn from(record_number: u64) -> Self {
        MftRecordNumber { record_number }
    }
}
impl Deref for MftRecordNumber {
    type Target = u64;

    fn deref(&self) -> &Self::Target {
        &self.record_number
    }
}