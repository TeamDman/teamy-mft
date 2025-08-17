use crate::mft::mft_record_attribute::MftRecordAttribute;
use eyre::bail;
use std::ops::Deref;

#[derive(Clone, Copy, Debug)]
pub struct MftRecordAttributeNonResidentHeader<'a> {
    mft_record_attribute: &'a MftRecordAttribute<'a>,
}
impl<'a> MftRecordAttributeNonResidentHeader<'a> {
    pub fn new(mft_record_attribute: &'a MftRecordAttribute<'a>) -> Self {
        Self {
            mft_record_attribute,
        }
    }
    #[inline(always)]
    pub fn starting_vcn(&self) -> u64 {
        u64::from_le_bytes(self[0x10..0x18].try_into().unwrap())
    }
    #[inline(always)]
    pub fn last_vcn(&self) -> u64 {
        u64::from_le_bytes(self[0x18..0x20].try_into().unwrap())
    }
    #[inline(always)]
    pub fn runlist_offset(&self) -> u16 {
        u16::from_le_bytes(self[0x20..0x22].try_into().unwrap())
    }
    #[inline(always)]
    pub fn compression_unit(&self) -> u16 {
        u16::from_le_bytes(self[0x22..0x24].try_into().unwrap())
    }
    #[inline(always)]
    pub fn allocated_size(&self) -> u64 {
        u64::from_le_bytes(self[0x28..0x30].try_into().unwrap())
    }
    #[inline(always)]
    pub fn real_size(&self) -> u64 {
        u64::from_le_bytes(self[0x30..0x38].try_into().unwrap())
    }
    #[inline(always)]
    pub fn initialized_size(&self) -> u64 {
        u64::from_le_bytes(self[0x38..0x40].try_into().unwrap())
    }
    pub fn runlist(&self) -> eyre::Result<&'a [u8]> {
        let off = self.runlist_offset() as usize;
        if off >= self.len() {
            bail!(
                "Runlist offset {} beyond attribute length {}",
                off,
                self.len()
            );
        }
        Ok(&self.mft_record_attribute.raw_data()[off..])
    }
}
impl<'a> Deref for MftRecordAttributeNonResidentHeader<'a> {
    type Target = MftRecordAttribute<'a>;

    fn deref(&self) -> &Self::Target {
        self.mft_record_attribute
    }
}
