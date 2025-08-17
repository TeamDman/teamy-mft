use crate::mft::mft_record_location::MftRecordLocationOnDisk;
use crate::mft::mft_record_number::MftRecordNumber;
use crate::mft::ntfs_boot_sector::NtfsBootSector;
use std::ops::Add;
use std::ops::Deref;

#[derive(Debug)]
pub struct MftLocationOnDisk {
    offset: u64,
}
impl From<&NtfsBootSector> for MftLocationOnDisk {
    fn from(value: &NtfsBootSector) -> Self {
        Self {
            offset: value.mft_cluster_number() * value.bytes_per_cluster(),
        }
    }
}
impl Deref for MftLocationOnDisk {
    type Target = u64;
    fn deref(&self) -> &Self::Target {
        &self.offset
    }
}
impl Add<MftRecordNumber> for MftLocationOnDisk {
    type Output = MftRecordLocationOnDisk;

    fn add(self, other: MftRecordNumber) -> Self::Output {
        MftRecordLocationOnDisk::new(self.offset + *other)
    }
}
