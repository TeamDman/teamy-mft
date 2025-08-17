use eyre::Result;
use eyre::eyre;

/// Generic decoded run list entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MftRecordAttributeRunListEntry {
    pub length_clusters: u64,
    // If none, the run is sparse
    pub local_cluster_network_start: Option<u64>,
}

/// Borrowed view over an encoded NTFS run list (sequence of data runs) used by any non-resident attribute.
#[derive(Debug, Clone, Copy)]
pub struct MftRecordAttributeRunList<'a> {
    raw: &'a [u8],
}

impl<'a> MftRecordAttributeRunList<'a> {
    pub fn new(raw: &'a [u8]) -> Self {
        Self { raw }
    }
    pub fn as_slice(&self) -> &'a [u8] {
        self.raw
    }
    pub fn iter(&self) -> MftRecordAttributeRunListIter<'a> {
        MftRecordAttributeRunListIter {
            raw: self.raw,
            pos: 0,
            last_lcn: 0,
        }
    }
    pub fn decode_all(&self) -> Result<Vec<MftRecordAttributeRunListEntry>> {
        self.iter().collect()
    }
}

pub struct MftRecordAttributeRunListIter<'a> {
    raw: &'a [u8],
    pos: usize,
    last_lcn: i64,
}

impl<'a> Iterator for MftRecordAttributeRunListIter<'a> {
    type Item = Result<MftRecordAttributeRunListEntry>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.raw.len() {
            return None;
        }
        let header = self.raw[self.pos];
        if header == 0 {
            return None;
        }
        let offset_size = (header & 0xF0) >> 4;
        let length_size = header & 0x0F;
        if length_size == 0 {
            return Some(Err(eyre!("Zero length_size in run header")));
        }
        self.pos += 1;
        if self.pos + length_size as usize > self.raw.len() {
            return Some(Err(eyre!("Run length field exceeds buffer")));
        }
        let mut length = 0u64;
        for i in 0..length_size {
            length |= (self.raw[self.pos + i as usize] as u64) << (8 * i);
        }
        self.pos += length_size as usize;
        let lcn_opt = if offset_size == 0 {
            None
        } else {
            if self.pos + offset_size as usize > self.raw.len() {
                return Some(Err(eyre!("Run offset field exceeds buffer")));
            }
            let mut delta: i64 = 0;
            for i in 0..offset_size {
                delta |= (self.raw[self.pos + i as usize] as i64) << (8 * i);
            }
            let sign_bit = 1i64 << (offset_size * 8 - 1);
            if delta & sign_bit != 0 {
                let mask = (!0i64) << (offset_size * 8);
                delta |= mask;
            }
            self.pos += offset_size as usize;
            self.last_lcn = self.last_lcn.wrapping_add(delta);
            Some(self.last_lcn as u64)
        };
        Some(Ok(MftRecordAttributeRunListEntry {
            length_clusters: length,
            local_cluster_network_start: lcn_opt,
        }))
    }
}
