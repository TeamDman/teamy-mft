use crate::mft::mft_location::MftLocationOnDisk;
use crate::ntfs::ntfs_drive_handle::NtfsDriveHandle;
use teamy_windows::file::HandleReadExt;
use uom::si::information::byte;
use uom::si::usize::Information;

pub struct NtfsBootSector {
    pub data: [u8; 512],
}
impl NtfsBootSector {
    /// Read the NTFS boot sector from the given drive handle.
    ///
    /// # Errors
    ///
    /// Returns an error if the drive handle cannot be read.
    pub fn try_from_handle(drive_handle: &NtfsDriveHandle) -> eyre::Result<Self> {
        Ok(NtfsBootSector {
            data: {
                let mut data = [0u8; 512];
                drive_handle.try_read_exact(0, data.as_mut_slice())?;
                data
            },
        })
    }
    #[must_use]
    pub fn bytes_per_sector(&self) -> u16 {
        u16::from_le_bytes([self.data[0x0b], self.data[0x0c]])
    }
    #[must_use]
    pub fn sectors_per_cluster(&self) -> u8 {
        self.data[0x0d]
    }
    #[must_use]
    pub fn mft_cluster_number(&self) -> u64 {
        u64::from_le_bytes([
            self.data[0x30],
            self.data[0x31],
            self.data[0x32],
            self.data[0x33],
            self.data[0x34],
            self.data[0x35],
            self.data[0x36],
            self.data[0x37],
        ])
    }
    #[must_use]
    pub fn bytes_per_cluster(&self) -> usize {
        self.bytes_per_sector() as usize * self.sectors_per_cluster() as usize
    }
    /// Returns the size of a single MFT file record as Information (bytes).
    /// Per NTFS spec, at offset 0x40 there is a signed byte:
    /// - If negative, the record size is 2^abs(value) bytes.
    /// - If non-negative, it is `clusters_per_file_record` * `bytes_per_cluster`.
    ///
    /// # Panics
    ///
    /// Panics if the exponent or cluster count fails to fit into `usize` (should not happen for valid sectors).
    #[must_use]
    pub fn file_record_size(&self) -> Information {
        let val = i8::from_ne_bytes([self.data[0x40]]);
        let bytes = if val < 0 {
            let shift =
                usize::try_from(-isize::from(val)).expect("record size exponent fits in usize");
            (1usize) << shift
        } else {
            let cluster_count = usize::try_from(val).expect("cluster count fits in usize");
            cluster_count * self.bytes_per_cluster()
        };
        Information::new::<byte>(bytes)
    }
    #[must_use]
    pub fn mft_location(&self) -> MftLocationOnDisk {
        self.into()
    }
}
impl std::fmt::Debug for NtfsBootSector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NtfsBootSector")
            .field("bytes_per_sector", &self.bytes_per_sector())
            .field("sectors_per_cluster", &self.sectors_per_cluster())
            .field("mft_cluster_number", &self.mft_cluster_number())
            .field("file_record_size", &self.file_record_size())
            .field("mft_location", &self.mft_location())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_boot_sector(
        bytes_per_sector: u16,
        sectors_per_cluster: u8,
        mft_cluster: u64,
        rec_byte: i8,
    ) -> NtfsBootSector {
        let mut bs = NtfsBootSector { data: [0u8; 512] };
        // bytes per sector @ 0x0B
        bs.data[0x0b] = (bytes_per_sector & 0xFF) as u8;
        bs.data[0x0c] = (bytes_per_sector >> 8) as u8;
        // sectors per cluster @ 0x0D
        bs.data[0x0d] = sectors_per_cluster;
        // mft cluster number @ 0x30
        bs.data[0x30..0x38].copy_from_slice(&mft_cluster.to_le_bytes());
        // clusters per file record @ 0x40 (signed)
        bs.data[0x40] = rec_byte as u8;
        bs
    }

    #[test]
    fn bytes_per_file_record_negative_exponent() {
        // rec_byte = -10 => 2^10 = 1024 bytes
        let bs = mk_boot_sector(512, 8, 100, -10);
        assert_eq!(bs.bytes_per_cluster(), 512 * 8);
        assert_eq!(bs.file_record_size().get::<byte>(), 1024);
    }

    #[test]
    fn bytes_per_file_record_positive_clusters() {
        // rec_byte = 2 => 2 clusters => 2 * bytes_per_cluster
        let bs = mk_boot_sector(512, 4, 200, 2);
        assert_eq!(bs.bytes_per_cluster(), 2048);
        assert_eq!(bs.file_record_size().get::<byte>(), 4096);
    }
}
