use crate::mft::mft_record_attribute::MftRecordAttribute;
use crate::mft::mft_record_attribute::NonResidentHeader;
use eyre::{Result, bail, eyre};

/// Wrapper specific to a type 0x80 ($DATA) attribute.
/// Exposes helpers for resident / non-resident variants.
#[derive(Clone, Copy, Debug)]
pub struct MftRecordX80DollarDataAttribute<'a> {
    inner: MftRecordAttribute<'a>,
}

impl<'a> MftRecordX80DollarDataAttribute<'a> {
    pub fn new(attr: MftRecordAttribute<'a>) -> Result<Self> {
        if attr.get_attr_type() != MftRecordAttribute::TYPE_DOLLAR_DATA {
            bail!(
                "Attribute type {:X} is not DATA (0x80)",
                attr.get_attr_type()
            );
        }
        Ok(Self { inner: attr })
    }
    
    #[inline(always)]
    pub fn inner(&self) -> MftRecordAttribute<'a> {
        self.inner
    }
    
    #[inline(always)]
    pub fn is_non_resident(&self) -> bool {
        self.inner.get_is_non_resident()
    }
    
    #[inline(always)]
    pub fn non_resident_header(&self) -> Option<NonResidentHeader<'_>> {
        self.inner.get_non_resident_header()
    }
    
    #[inline(always)]
    pub fn resident_payload(&self) -> Option<&'a [u8]> {
        self.inner.get_resident_content()
    }
    
    pub fn get_data_run_list(&self) -> Result<Option<DataRunList<'a>>> {
        if !self.is_non_resident() { return Ok(None); }
        let raw = self.inner.get_raw();
        if raw.len() < 0x40 { bail!("DATA attribute too short for non-resident header (len={})", raw.len()); }
        let runlist_off = u16::from_le_bytes(raw[0x20..0x22].try_into().unwrap()) as usize;
        if runlist_off >= raw.len() { bail!("Runlist offset {} beyond attribute length {}", runlist_off, raw.len()); }
        Ok(Some(DataRunList { raw: &raw[runlist_off..] }))
    }
}

/// Represents a raw encoded runlist (sequence of data runs) for a non-resident attribute.
#[derive(Clone, Copy, Debug)]
pub struct DataRunList<'a> { pub(crate) raw: &'a [u8] }
impl<'a> DataRunList<'a> {
    pub fn as_slice(&self) -> &'a [u8] { self.raw }
    pub fn iter(&self) -> DataRunIter<'a> { DataRunIter { raw: self.raw, pos: 0, last_lcn: 0 } }
    pub fn decode_all(&self) -> Result<Vec<DataRunEntry>> { self.iter().collect() }
}

/// A single decoded data run. If lcn_start is None, the run is sparse (logical zeros, no clusters allocated).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataRunEntry { pub length_clusters: u64, pub lcn_start: Option<u64> }

pub struct DataRunIter<'a> { raw: &'a [u8], pos: usize, last_lcn: i64 }
impl<'a> Iterator for DataRunIter<'a> {
    type Item = Result<DataRunEntry>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.raw.len() { return None; }
        let header = self.raw[self.pos];
        if header == 0 { return None; }
        let offset_size = (header & 0xF0) >> 4;
        let length_size = header & 0x0F;
        if length_size == 0 { return Some(Err(eyre!("Zero length_size in run header"))); }
        self.pos += 1;
        if self.pos + length_size as usize > self.raw.len() { return Some(Err(eyre!("Run length field exceeds buffer"))); }
        let mut length = 0u64;
        for i in 0..length_size { length |= (self.raw[self.pos + i as usize] as u64) << (8 * i); }
        self.pos += length_size as usize;
        // Offset (may be 0 size meaning sparse)
        let lcn_opt = if offset_size == 0 { None } else {
            if self.pos + offset_size as usize > self.raw.len() { return Some(Err(eyre!("Run offset field exceeds buffer"))); }
            let mut delta: i64 = 0;
            for i in 0..offset_size { delta |= (self.raw[self.pos + i as usize] as i64) << (8 * i); }
            // sign extend
            let sign_bit = 1i64 << (offset_size*8 - 1);
            if delta & sign_bit != 0 { let mask = (!0i64) << (offset_size*8); delta |= mask; }
            self.pos += offset_size as usize;
            self.last_lcn = self.last_lcn.wrapping_add(delta);
            Some(self.last_lcn as u64)
        };
        Some(Ok(DataRunEntry { length_clusters: length, lcn_start: lcn_opt }))
    }
}
