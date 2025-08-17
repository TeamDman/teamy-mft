use eyre::{bail, Result};
use crate::mft::mft_record_attribute::{MftRecordAttribute, NonResidentHeader};

/// Wrapper specific to a type 0x80 ($DATA) attribute.
/// Exposes helpers for resident / non-resident variants.
#[derive(Clone, Copy, Debug)]
pub struct MftRecordX80DataAttribute<'a> { inner: MftRecordAttribute<'a> }

impl<'a> MftRecordX80DataAttribute<'a> {
    pub fn new(attr: MftRecordAttribute<'a>) -> Result<Self> {
        if attr.get_attr_type() != MftRecordAttribute::TYPE_DATA { bail!("Attribute type {:X} is not DATA (0x80)", attr.get_attr_type()); }
        Ok(Self { inner: attr })
    }
    #[inline(always)] pub fn inner(&self) -> MftRecordAttribute<'a> { self.inner }
    #[inline(always)] pub fn is_non_resident(&self) -> bool { self.inner.get_is_non_resident() }
    pub fn non_resident_header(&self) -> Option<NonResidentHeader<'_>> { self.inner.get_non_resident_header() }
    pub fn resident_payload(&self) -> Option<&'a [u8]> { self.inner.get_resident_content() }
    pub fn runlist(&self) -> Result<Option<&'a [u8]>> {
    if !self.is_non_resident() { return Ok(None); }
    let raw = self.inner.get_raw();
    if raw.len() < 0x40 { bail!("DATA attribute too short for non-resident header (len={})", raw.len()); }
    let runlist_off = u16::from_le_bytes(raw[0x20..0x22].try_into().unwrap()) as usize;
    if runlist_off >= raw.len() { bail!("Runlist offset {} beyond attribute length {}", runlist_off, raw.len()); }
    Ok(Some(&raw[runlist_off..]))
    }
}
