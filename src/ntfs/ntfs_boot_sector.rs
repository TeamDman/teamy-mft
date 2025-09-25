use crate::mft::mft_location::MftLocationOnDisk;
use crate::ntfs::ntfs_drive_handle::NtfsDriveHandle;
use teamy_windows::file::HandleReadExt;

pub struct NtfsBootSector {
    pub data: [u8; 512],
}
impl NtfsBootSector {
    pub fn try_from_handle(drive_handle: &NtfsDriveHandle) -> eyre::Result<Self> {
        Ok(NtfsBootSector {
            data: {
                let mut data = [0u8; 512];
                drive_handle.try_read_exact(0, data.as_mut_slice())?;
                data
            },
        })
    }
    pub fn bytes_per_sector(&self) -> u16 {
        u16::from_le_bytes([self.data[0x0b], self.data[0x0c]])
    }
    pub fn sectors_per_cluster(&self) -> u8 {
        self.data[0x0d]
    }
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
    pub fn bytes_per_cluster(&self) -> u64 {
        self.bytes_per_sector() as u64 * self.sectors_per_cluster() as u64
    }
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
            .field("mft_location", &self.mft_location())
            .finish()
    }
}
