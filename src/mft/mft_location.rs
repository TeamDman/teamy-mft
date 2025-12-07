use crate::mft::mft_record_location::MftRecordLocationOnDisk;
use crate::mft::mft_record_number::MftRecordNumber;
use crate::ntfs::ntfs_boot_sector::NtfsBootSector;
use std::ops::Deref;
use uom::si::information::byte;
use uom::si::usize::Information;

#[derive(Debug)]
pub struct MftLocationOnDisk {
    offset: Information,
}
impl From<&NtfsBootSector> for MftLocationOnDisk {
    fn from(value: &NtfsBootSector) -> Self {
        let ntfs_cluster_size = Information::new::<byte>(value.bytes_per_cluster());
        Self {
            offset: value.mft_cluster_number() as usize * ntfs_cluster_size,
        }
    }
}
impl Deref for MftLocationOnDisk {
    type Target = Information;
    fn deref(&self) -> &Self::Target {
        &self.offset
    }
}
impl MftLocationOnDisk {
    /// Compute the on-disk byte location of a given MFT record number.
    /// `bytes_per_record` must be provided explicitly (do not assume cluster size).
    #[must_use] 
    pub fn record_location(
        &self,
        record_number: MftRecordNumber,
        bytes_per_record: Information,
    ) -> MftRecordLocationOnDisk {
        // offset + (record_number * bytes_per_record)
        MftRecordLocationOnDisk::new(self.offset + (*record_number as usize * bytes_per_record))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_location_scales_by_bytes_per_record() {
        // base offset = 1 MiB
        let base = Information::new::<byte>(1_048_576);
        let mft_loc = MftLocationOnDisk { offset: base };

        let bpr = Information::new::<byte>(1024); // 1 KiB records
        let n0 = MftRecordNumber::new(0);
        let n1 = MftRecordNumber::new(1);
        let n5 = MftRecordNumber::new(5);

        let loc0 = mft_loc.record_location(n0, bpr);
        let loc1 = mft_loc.record_location(n1, bpr);
        let loc5 = mft_loc.record_location(n5, bpr);

        assert_eq!(*loc0, base);
        assert_eq!(*loc1, base + bpr);
        assert_eq!(*loc5, base + 5usize * bpr);
    }
}
