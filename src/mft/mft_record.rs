use crate::mft::mft_record_attribute::MftRecordAttribute;
use crate::mft::mft_record_attribute_iter::MftRecordAttributeIter;
use crate::mft::mft_record_location::MftRecordLocationOnDisk;
use crate::mft::mft_record_number::MftRecordNumber;
use crate::windows::win_handles::AutoClosingHandle;
use eyre::bail;
use eyre::eyre;

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
    #[inline(always)]
    pub fn get_update_sequence_array_offset(&self) -> u16 {
        self.read_u16(Self::OFFSET_FOR_UPDATE_SEQUENCE_ARRAY_OFFSET)
    }

    /// Number of 2-byte elements in the USA, including the first sentinel value.
    #[inline(always)]
    pub fn get_update_sequence_array_size_words(&self) -> u16 {
        self.read_u16(Self::OFFSET_FOR_UPDATE_SEQUENCE_ARRAY_SIZE)
    }

    /// $LogFile sequence number (LSN) at offset 0x08 (8 bytes LE).
    #[inline(always)]
    pub fn get_dollar_log_file(&self) -> u64 {
        self.read_u64(Self::OFFSET_FOR_LOGFILE_SEQUENCE_NUMBER)
    }

    #[inline(always)]
    pub fn get_sequence_number(&self) -> u16 {
        self.read_u16(Self::OFFSET_FOR_SEQUENCE)
    }

    #[inline(always)]
    pub fn get_hard_link_count(&self) -> u16 {
        self.read_u16(Self::OFFSET_FOR_HARDLINKS)
    }

    #[inline(always)]
    pub fn get_first_attribute_offset(&self) -> u16 {
        self.read_u16(Self::OFFSET_FOR_FIRST_ATTR)
    }

    #[inline(always)]
    pub fn get_flags(&self) -> u16 {
        self.read_u16(Self::OFFSET_FOR_FLAGS)
    }

    /// Bytes in use inside this record.
    #[inline(always)]
    pub fn get_used_size(&self) -> u32 {
        self.read_u32(Self::OFFSET_FOR_USED_SIZE)
    }

    /// Allocated size (record size, typically 1024 or 4096).
    #[inline(always)]
    pub fn get_allocated_size(&self) -> u32 {
        self.read_u32(Self::OFFSET_FOR_ALLOC_SIZE)
    }

    /// Base record reference (8 bytes) – if non-zero, this is an extension record.
    #[inline(always)]
    pub fn get_base_reference_raw(&self) -> u64 {
        self.read_u64(Self::OFFSET_FOR_BASE_REF)
    }

    #[inline(always)]
    pub fn get_next_attribute_id(&self) -> u16 {
        self.read_u16(Self::OFFSET_FOR_NEXT_ATTR_ID)
    }

    #[inline(always)]
    pub fn get_record_number(&self) -> MftRecordNumber {
        self.read_u32(Self::OFFSET_FOR_RECORD_NUMBER).into()
    }

    /// On success returns Ok(()). Any integrity issue yields an Err with context.
    pub fn apply_update_sequence_array_fixups(&mut self) -> eyre::Result<()> {
        if self.get_signature() != b"FILE" {
            bail!(
                "Cannot apply fixups: signature is not FILE (found {:?})",
                self.get_signature()
            );
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
            if end > self.data.len() {
                // partial final stride ok
                break;
            }
            let sector_last_two = &mut self.data[end - 2..end];
            if sector_last_two != update_sequence {
                bail!(
                    "Fixup mismatch at stride {}: expected {:02X?} found {:02X?}",
                    i,
                    update_sequence,
                    sector_last_two
                );
            }
            sector_last_two.copy_from_slice(replacement);
        }
        Ok(())
    }

    /// Iterate raw attribute slices (header + body) in this record.
    pub fn iter_raw_attributes(&self) -> MftRecordAttributeIter<'_> {
        let start = self.get_first_attribute_offset() as usize;
        let used = self.get_used_size() as usize;
        debug_assert!(
            start < used,
            "Attribute start {} must be less than used size {}",
            start,
            used
        );
        debug_assert!(
            used <= self.data.len(),
            "Used size {} must not exceed record length {}",
            used,
            self.data.len()
        );
        MftRecordAttributeIter {
            data: &self.data,
            pos: start,
            used,
        }
    }

    /// Find the first non-resident $DATA (0x80) attribute and return its full slice.
    pub fn find_non_resident_data_attribute(&self) -> Option<MftRecordAttribute<'_>> {
        self.iter_raw_attributes()
            .find(|a| a.get_attr_type() == MftRecordAttribute::TYPE_DATA && a.get_is_non_resident())
    }

    /// Iterate over all $DATA (0x80) attributes (resident and non-resident)
    pub fn iter_data_attributes(&self) -> impl Iterator<Item = MftRecordAttribute<'_>> + '_ {
        self.iter_raw_attributes()
            .filter(|a| a.get_attr_type() == MftRecordAttribute::TYPE_DATA)
    }

    /// Collect runlists for all non-resident DATA attributes (multiple segments if Attribute List used later).
    pub fn get_all_data_attribute_runlists(&self) -> eyre::Result<Vec<&[u8]>> {
        let mut out = Vec::new();
        // Manual scan like get_data_attribute_runlist but collecting all.
        let start = self.get_first_attribute_offset() as usize;
        let used = (self.get_used_size() as usize).min(self.data.len());
        let mut pos = start;
        while pos + 8 <= used {
            let attr_type = u32::from_le_bytes(self.data[pos..pos+4].try_into().unwrap());
            if attr_type == MftRecordAttribute::TYPE_END { break; }
            let attr_len = u32::from_le_bytes(self.data[pos+4..pos+8].try_into().unwrap()) as usize;
            if attr_len == 0 || pos + attr_len > used { break; }
            let non_resident_flag = self.data.get(pos+8).copied().unwrap_or(0);
            if attr_type == MftRecordAttribute::TYPE_DATA && non_resident_flag != 0 {
                if attr_len < 0x40 { bail!("DATA attribute too short for non-resident header (len={})", attr_len); }
                let runlist_off = u16::from_le_bytes(self.data[pos+0x20..pos+0x22].try_into().unwrap()) as usize;
                if runlist_off >= attr_len { bail!("Runlist offset {} beyond attribute length {}", runlist_off, attr_len); }
                out.push(&self.data[pos + runlist_off .. pos + attr_len]);
            }
            pos += attr_len;
        }
        if out.is_empty() { bail!("No non-resident $DATA attributes found"); }
        Ok(out)
    }

    /// Extract the runlist slice (data runs) from the first non-resident $DATA attribute.
    pub fn get_data_attribute_runlist(&self) -> eyre::Result<&[u8]> {
        // Manually search again to obtain raw slice lifetime directly from self.data.
        let start = self.get_first_attribute_offset() as usize;
        let used = self.get_used_size() as usize;
        debug_assert!(used <= self.data.len());
        let mut pos = start;
        while pos + 8 <= used {
            let attr_type = u32::from_le_bytes(self.data[pos..pos + 4].try_into().unwrap());
            if attr_type == MftRecordAttribute::TYPE_END {
                break;
            }
            let attr_len =
                u32::from_le_bytes(self.data[pos + 4..pos + 8].try_into().unwrap()) as usize;
            if attr_len == 0 || pos + attr_len > used {
                break;
            }
            if attr_type == MftRecordAttribute::TYPE_DATA
                && self.data.get(pos + 8).copied().unwrap_or(0) != 0
            {
                // non-resident DATA
                if attr_len < 0x40 {
                    bail!(
                        "DATA attribute too short for non-resident header (len={})",
                        attr_len
                    );
                }
                let runlist_off =
                    u16::from_le_bytes(self.data[pos + 0x20..pos + 0x22].try_into().unwrap())
                        as usize;
                if runlist_off >= attr_len {
                    bail!(
                        "Runlist offset {} beyond attribute length {}",
                        runlist_off,
                        attr_len
                    );
                }
                let run_slice = &self.data[pos + runlist_off..pos + attr_len];
                return Ok(run_slice);
            }
            pos += attr_len;
        }
        Err(eyre!("No non-resident $DATA attribute found"))
    }
}
