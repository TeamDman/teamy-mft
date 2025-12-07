use crate::mft::mft_record::MftRecord;
use crate::mft::mft_record_attribute::MftRecordAttribute;

#[derive(Debug)]
pub struct MftRecordAttributeIter<'a> {
    mft_record: &'a MftRecord,
    position: usize,
    mft_record_used_size: usize,
}
impl<'a> MftRecordAttributeIter<'a> {
    pub fn new(mft_record: &'a MftRecord) -> Self {
        let first_mft_attribute_offset = mft_record.get_first_attribute_offset() as usize;
        let mft_record_used_size = mft_record.get_used_size() as usize;
        debug_assert!(
            first_mft_attribute_offset < mft_record_used_size,
            "Attribute start {first_mft_attribute_offset} must be less than used size {mft_record_used_size}"
        );
        debug_assert!(
            mft_record_used_size <= mft_record.len(),
            "Used size {} must not exceed record length {}",
            mft_record_used_size,
            mft_record.len()
        );
        Self {
            mft_record,
            position: first_mft_attribute_offset,
            mft_record_used_size,
        }
    }
}
impl<'a> Iterator for MftRecordAttributeIter<'a> {
    type Item = MftRecordAttribute<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.position + 4 > self.mft_record_used_size {
            return None;
        }
        let attr_type = u32::from_le_bytes(
            self.mft_record[self.position..self.position + 4]
                .try_into()
                .ok()?,
        );
        if attr_type == MftRecordAttribute::TYPE_END {
            return None;
        }
        if self.position + 8 > self.mft_record_used_size {
            return None;
        }
        let attr_len = u32::from_le_bytes(
            self.mft_record[self.position + 4..self.position + 8]
                .try_into()
                .ok()?,
        ) as usize;
        if attr_len == 0 || self.position + attr_len > self.mft_record_used_size {
            return None;
        }
        let start = self.position;
        let end = start + attr_len;
        self.position = end;
        let raw = &self.mft_record[start..end];
        MftRecordAttribute::new(raw).ok()
    }
}
