use eyre::bail;
use eyre::eyre;

/// Wrapper around a borrowed attribute slice inside an MFT record.
/// Provides typed accessors for common header fields without copying.
#[derive(Clone, Copy, Debug)]
pub struct MftRecordAttribute<'a> {
    pub(crate) raw: &'a [u8],
}

impl<'a> MftRecordAttribute<'a> {
    pub const TYPE_END: u32 = 0xFFFF_FFFF;
    pub const TYPE_DATA: u32 = 0x80;

    pub fn from_raw(raw: &'a [u8]) -> eyre::Result<Self> {
        if raw.len() < 16 {
            bail!("Attribute slice too short ({} bytes)", raw.len());
        }
        Ok(Self { raw })
    }
    #[inline(always)]
    pub fn get_raw(&self) -> &'a [u8] {
        self.raw
    }

    #[inline(always)]
    pub fn get_attr_type(&self) -> u32 {
        u32::from_le_bytes(self.raw[0..4].try_into().unwrap())
    }

    #[inline(always)]
    pub fn get_total_length(&self) -> u32 {
        u32::from_le_bytes(self.raw[4..8].try_into().unwrap())
    }

    #[inline(always)]
    pub fn get_is_non_resident(&self) -> bool {
        self.raw[8] != 0
    }

    #[inline(always)]
    pub fn get_name_len(&self) -> u8 {
        self.raw[9]
    }

    #[inline(always)]
    pub fn get_name_offset(&self) -> u16 {
        u16::from_le_bytes(self.raw[10..12].try_into().unwrap())
    }

    #[inline(always)]
    pub fn get_flags(&self) -> u16 {
        u16::from_le_bytes(self.raw[12..14].try_into().unwrap())
    }
    
    #[inline(always)]
    pub fn get_attr_id(&self) -> u16 {
        u16::from_le_bytes(self.raw[14..16].try_into().unwrap())
    }

    // Resident specific
    pub fn get_resident_content(&self) -> Option<&'a [u8]> {
        if self.get_is_non_resident() || self.raw.len() < 0x18 {
            return None;
        }
        let size = u32::from_le_bytes(self.raw[0x10..0x14].try_into().ok()?) as usize;
        let off = u16::from_le_bytes(self.raw[0x14..0x16].try_into().ok()?) as usize;
        if off + size > self.raw.len() {
            return None;
        }
        Some(&self.raw[off..off + size])
    }

    // Non-resident specific
    pub fn get_non_resident_header(&self) -> Option<NonResidentHeader<'_>> {
        if !self.get_is_non_resident() || self.raw.len() < 0x40 {
            return None;
        }
        Some(NonResidentHeader { raw: self.raw })
    }

        pub fn as_x80(&self) -> Option<crate::mft::mft_record_attribute_x80_data_attribute::MftRecordX80DataAttribute<'a>> {
            if self.get_attr_type() == Self::TYPE_DATA {
                crate::mft::mft_record_attribute_x80_data_attribute::MftRecordX80DataAttribute::new(*self).ok()
            } else { None }
        }
}

#[derive(Clone, Copy, Debug)]
pub struct NonResidentHeader<'a> {
    raw: &'a [u8],
}
impl<'a> NonResidentHeader<'a> {
    #[inline(always)]
    pub fn starting_vcn(&self) -> u64 {
        u64::from_le_bytes(self.raw[0x10..0x18].try_into().unwrap())
    }
    #[inline(always)]
    pub fn last_vcn(&self) -> u64 {
        u64::from_le_bytes(self.raw[0x18..0x20].try_into().unwrap())
    }
    #[inline(always)]
    pub fn runlist_offset(&self) -> u16 {
        u16::from_le_bytes(self.raw[0x20..0x22].try_into().unwrap())
    }
    #[inline(always)]
    pub fn compression_unit(&self) -> u16 {
        u16::from_le_bytes(self.raw[0x22..0x24].try_into().unwrap())
    }
    #[inline(always)]
    pub fn allocated_size(&self) -> u64 {
        u64::from_le_bytes(self.raw[0x28..0x30].try_into().unwrap())
    }
    #[inline(always)]
    pub fn real_size(&self) -> u64 {
        u64::from_le_bytes(self.raw[0x30..0x38].try_into().unwrap())
    }
    #[inline(always)]
    pub fn initialized_size(&self) -> u64 {
        u64::from_le_bytes(self.raw[0x38..0x40].try_into().unwrap())
    }
    pub fn runlist(&self) -> eyre::Result<&'a [u8]> {
        let off = self.runlist_offset() as usize;
        if off >= self.raw.len() {
            return Err(eyre!(
                "Runlist offset {} beyond attribute length {}",
                off,
                self.raw.len()
            ));
        }
        Ok(&self.raw[off..])
    }
}
