use crate::mft::mft_record_location::MftRecordLocationOnDisk;
use crate::windows::win_handles::AutoClosingHandle;
use eyre::bail;

/// https://digitalinvestigator.blogspot.com/2022/03/the-ntfs-master-file-table-mft.html?m=1
/// "On a standard hard drive with 512-byte sectors, the MFT is structured as a series of 1,024-byte records,
/// also known as “entries,” one for each file and directory on a volume but only the first 42 bytes (MFT header)
/// have a defined purpose. The remaining 982 bytes store attributes, which are small data structures that have
/// a very specific purpose. However, on advanced format (AF) drives with 4KB sectors,
/// each MFT record will be 4,096 bytes instead."
pub const MFT_RECORD_SIZE: u16 = 1024;

pub struct MftRecord {
    pub data: [u8; MFT_RECORD_SIZE as usize],
}
impl MftRecord {
    pub fn from_data(data: [u8; MFT_RECORD_SIZE as usize]) -> Self {
        Self { data }
    }
    pub fn try_from_handle(
        drive_handle: &AutoClosingHandle,
        mft_record_location: MftRecordLocationOnDisk,
    ) -> eyre::Result<Self> {
        let mut data = [0u8; MFT_RECORD_SIZE as usize];
        drive_handle.try_read_exact(*mft_record_location as i64, data.as_mut_slice())?;
        if &data[0..4] != b"FILE" {
            bail!(
                "Invalid MFT record signature: expected 'FILE', got {:?}",
                String::from_utf8_lossy(&data[0..4])
            );
        }
        Ok(Self { data })
    }
}