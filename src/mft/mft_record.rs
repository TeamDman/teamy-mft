use crate::mft::mft_record_location::MftRecordLocationOnDisk;
use crate::mft::mft_record_number::MftRecordNumber;
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
    // ---- Raw field offset constants (for clarity & reuse) ----
    const OFFSET_FOR_SIGNATURE: usize = 0x00;
    const OFFSET_FOR_UPDATE_SEQUENCE_ARRAY_OFFSET: usize = 0x04; // u16
    const OFFSET_FOR_UPDATE_SEQUENCE_ARRAY_SIZE: usize = 0x06; // u16 (count of 2-byte words)
    const OFFSET_FOR_LOGFILE_SEQUENCE_NUMBER: usize = 0x08; // u64 ($LogFile sequence number / LSN)
    const OFFSET_FOR_SEQUENCE: usize = 0x10; // u16
    const OFFSET_FOR_HARDLINKS: usize = 0x12; // u16
    const OFFSET_FOR_FIRST_ATTR: usize = 0x14; // u16
    const OFFSET_FOR_FLAGS: usize = 0x16; // u16
    const OFFSET_FOR_USED_SIZE: usize = 0x18; // u32
    const OFFSET_FOR_ALLOC_SIZE: usize = 0x1C; // u32
    const OFFSET_FOR_BASE_REF: usize = 0x20; // u64
    const OFFSET_FOR_NEXT_ATTR_ID: usize = 0x28; // u16
    // 0x2A padding
    const OFFSET_FOR_RECORD_NUMBER: usize = 0x2C; // u32 on-disk

    /// Zero-copy access to the 4-byte signature.
    pub fn get_signature(&self) -> &[u8; 4] {
        // SAFETY: first 4 bytes always exist; casting to fixed array reference.
        unsafe { &*(self.data.as_ptr().add(Self::OFFSET_FOR_SIGNATURE) as *const [u8; 4]) }
    }

    #[inline(always)]
    fn read_u16(&self, off: usize) -> u16 {
        // SAFETY: Bounds ensured by caller placement; use unaligned read then convert LE.
        unsafe {
            u16::from_le(std::ptr::read_unaligned(
                self.data.as_ptr().add(off) as *const u16
            ))
        }
    }
    #[inline(always)]
    fn read_u32(&self, off: usize) -> u32 {
        unsafe {
            u32::from_le(std::ptr::read_unaligned(
                self.data.as_ptr().add(off) as *const u32
            ))
        }
    }
    #[inline(always)]
    fn read_u64(&self, off: usize) -> u64 {
        unsafe {
            u64::from_le(std::ptr::read_unaligned(
                self.data.as_ptr().add(off) as *const u64
            ))
        }
    }

    /// Offset (in bytes from record start) to the Update Sequence Array (USA).
    /// NOTE: This was previously (incorrectly) read from 0x18/0x19 (used size field).
    /// Correct field per NTFS layout is at 0x04.
    pub fn get_update_sequence_array_offset(&self) -> u16 {
        self.read_u16(Self::OFFSET_FOR_UPDATE_SEQUENCE_ARRAY_OFFSET)
    }
    /// Number of 2-byte elements in the USA, including the first sentinel value.
    pub fn get_update_sequence_array_size_words(&self) -> u16 {
        self.read_u16(Self::OFFSET_FOR_UPDATE_SEQUENCE_ARRAY_SIZE)
    }
    /// $LogFile sequence number (LSN) at offset 0x08 (8 bytes LE).
    pub fn get_dollar_log_file(&self) -> u64 {
        self.read_u64(Self::OFFSET_FOR_LOGFILE_SEQUENCE_NUMBER)
    }
    pub fn get_sequence_number(&self) -> u16 {
        self.read_u16(Self::OFFSET_FOR_SEQUENCE)
    }
    pub fn get_hard_link_count(&self) -> u16 {
        self.read_u16(Self::OFFSET_FOR_HARDLINKS)
    }
    pub fn get_first_attribute_offset(&self) -> u16 {
        self.read_u16(Self::OFFSET_FOR_FIRST_ATTR)
    }
    pub fn get_flags(&self) -> u16 {
        self.read_u16(Self::OFFSET_FOR_FLAGS)
    }
    /// Bytes in use inside this record.
    pub fn get_used_size(&self) -> u32 {
        self.read_u32(Self::OFFSET_FOR_USED_SIZE)
    }
    /// Allocated size (record size, typically 1024 or 4096).
    pub fn get_allocated_size(&self) -> u32 {
        self.read_u32(Self::OFFSET_FOR_ALLOC_SIZE)
    }
    /// Base record reference (8 bytes) – if non-zero, this is an extension record.
    pub fn get_base_reference_raw(&self) -> u64 {
        self.read_u64(Self::OFFSET_FOR_BASE_REF)
    }
    pub fn get_next_attribute_id(&self) -> u16 {
        self.read_u16(Self::OFFSET_FOR_NEXT_ATTR_ID)
    }
    pub fn get_record_number(&self) -> MftRecordNumber {
        self.read_u32(Self::OFFSET_FOR_RECORD_NUMBER).into()
    }

    /// Apply USA fixups in-place. Returns true if all sectors validated, false if any mismatch.
    /// Safety: operates purely on the in-memory buffer; caller decides how to handle false.
    pub fn apply_fixups(&mut self) -> bool {
        if self.get_signature() != b"FILE" {
            return false;
        }
        let usa_offset = self.get_update_sequence_array_offset() as usize;
        let usa_size = self.get_update_sequence_array_size_words() as usize; // number of 2-byte words
        if usa_size < 2 {
            return false;
        }
        let fixup_bytes_len = usa_size * 2;
        if usa_offset == 0 || usa_offset + fixup_bytes_len > self.data.len() {
            return false;
        }
        let usa = self.data[usa_offset..usa_offset + fixup_bytes_len].to_vec();
        let update_sequence: [u8; 2] = usa[0..2].try_into().unwrap();
        let mut valid = true;
        for (i, replacement) in usa[2..].chunks_exact(2).enumerate() {
            let end = (i + 1) * 512; // stride size fixed at 512 logical bytes
            if end > self.data.len() {
                break;
            } // allow partial at tail
            let sector_last_two = &mut self.data[end - 2..end];
            if sector_last_two != update_sequence {
                valid = false;
                continue;
            }
            sector_last_two.copy_from_slice(replacement);
        }
        valid
    }
}
