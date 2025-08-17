use crate::mft::mft_record_attribute::MftRecordAttribute;

pub struct MftRecordAttributeIter<'a> {
    pub(crate) data: &'a [u8],
    pub(crate) pos: usize,
    pub(crate) used: usize,
}

impl<'a> Iterator for MftRecordAttributeIter<'a> {
    type Item = MftRecordAttribute<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.pos + 4 > self.used {
            return None;
        }
        let attr_type = u32::from_le_bytes(self.data[self.pos..self.pos + 4].try_into().ok()?);
        if attr_type == MftRecordAttribute::TYPE_END {
            return None;
        }
        if self.pos + 8 > self.used {
            return None;
        }
        let attr_len =
            u32::from_le_bytes(self.data[self.pos + 4..self.pos + 8].try_into().ok()?) as usize;
        if attr_len == 0 || self.pos + attr_len > self.used {
            return None;
        }
        let start = self.pos;
        let end = start + attr_len;
        self.pos = end;
        let raw = &self.data[start..end];
        MftRecordAttribute::from_raw(raw).ok()
    }
}
