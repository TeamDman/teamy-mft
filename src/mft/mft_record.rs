use crate::mft::mft_record_location::MftRecordLocationOnDisk;
use crate::mft::mft_record_number::MftRecordNumber;
use crate::windows::win_handles::AutoClosingHandle;
use eyre::{bail, eyre};

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

    /// Apply USA fixups in-place.
    /// On success returns Ok(()). Any integrity issue yields an Err with context.
    pub fn apply_fixups(&mut self) -> eyre::Result<()> {
        if self.get_signature() != b"FILE" {
            bail!("Cannot apply fixups: signature is not FILE (found {:?})", self.get_signature());
        }
        let usa_offset = self.get_update_sequence_array_offset() as usize;
        let usa_size_words = self.get_update_sequence_array_size_words() as usize; // total 2-byte words including sentinel
        if usa_size_words < 2 {
            bail!("Invalid USA: size words {} < 2", usa_size_words);
        }
        let fixup_bytes_len = usa_size_words * 2;
        if usa_offset == 0 || usa_offset + fixup_bytes_len > self.data.len() {
            bail!(
                "Invalid USA bounds: offset={} length={} record_len={}",
                usa_offset,
                fixup_bytes_len,
                self.data.len()
            );
        }
    let usa_vec = self.data[usa_offset..usa_offset + fixup_bytes_len].to_vec();
    let update_sequence = &usa_vec[0..2];
    let replacements = &usa_vec[2..];
    for (i, replacement) in replacements.chunks_exact(2).enumerate() {
            let end = (i + 1) * 512; // logical stride size
            if end > self.data.len() { break; } // partial final stride ok
            let sector_last_two = &mut self.data[end - 2..end];
            if sector_last_two != update_sequence {
                bail!("Fixup mismatch at stride {}: expected {:02X?} found {:02X?}", i, update_sequence, sector_last_two);
            }
            sector_last_two.copy_from_slice(replacement);
        }
        Ok(())
    }

    /// Iterate raw attribute slices (header + body) in this record.
    pub fn iter_raw_attributes(&self) -> RawAttributeIter<'_> {
        let start = self.get_first_attribute_offset() as usize;
        let used = self.get_used_size() as usize;
        let used = used.min(self.data.len());
        RawAttributeIter { data: &self.data, pos: start, used }
    }
    /// Find the first non-resident $DATA (0x80) attribute and return its full slice.
    pub fn find_non_resident_data_attribute(&self) -> Option<&[u8]> {
        for attr in self.iter_raw_attributes() {
            if attribute_type(attr) == 0x80 && is_non_resident(attr) { return Some(attr); }
        }
        None
    }
    /// Extract the runlist slice (data runs) from the first non-resident $DATA attribute.
    pub fn get_data_attribute_runlist(&self) -> eyre::Result<&[u8]> {
        let attr = self
            .find_non_resident_data_attribute()
            .ok_or_else(|| eyre!("No non-resident $DATA attribute found"))?;
        if attr.len() < 0x22 { bail!("Attribute too short to contain runlist header (len={})", attr.len()); }
        // non-resident header layout: runlist offset at +0x20 (u16)
        let runlist_off = u16::from_le_bytes([attr[0x20], attr[0x21]]) as usize;
        if runlist_off >= attr.len() { bail!("Runlist offset {} beyond attribute length {}", runlist_off, attr.len()); }
        let run_slice = &attr[runlist_off..];
        Ok(run_slice)
    }
}

/// Iterator over raw attribute byte slices inside an MFT record.
pub struct RawAttributeIter<'a> { data: &'a [u8], pos: usize, used: usize }
impl<'a> Iterator for RawAttributeIter<'a> {
    type Item = &'a [u8];
    fn next(&mut self) -> Option<Self::Item> {
        if self.pos + 4 > self.used { return None; }
        let attr_type = u32::from_le_bytes(self.data[self.pos..self.pos+4].try_into().ok()?);
        if attr_type == 0xFFFF_FFFF { return None; }
        if self.pos + 8 > self.used { return None; }
        let attr_len = u32::from_le_bytes(self.data[self.pos+4..self.pos+8].try_into().ok()?) as usize;
        if attr_len == 0 || self.pos + attr_len > self.used { return None; }
        let start = self.pos; let end = start + attr_len; self.pos = end; Some(&self.data[start..end])
    }
}

#[inline(always)]
fn attribute_type(attr: &[u8]) -> u32 { u32::from_le_bytes(attr[0..4].try_into().unwrap()) }
#[inline(always)]
fn is_non_resident(attr: &[u8]) -> bool { attr.get(8).map(|b| *b != 0).unwrap_or(false) }
