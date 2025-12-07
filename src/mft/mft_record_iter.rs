use crate::mft::mft_record::MftRecord;
use bytes::Bytes;
use uom::ConstZero;
use uom::si::information::byte;
use uom::si::usize::Information;

/// Zero-copy iterator over MFT records stored contiguously in a `Bytes` buffer.
#[derive(Debug)]
pub struct MftRecordIter {
    bytes: Bytes,
    entry_size: Information,
    index: usize,
    total: usize,
}

impl MftRecordIter {
    pub fn new(bytes: Bytes, entry_size: Information) -> Self {
        let total = if entry_size == Information::ZERO {
            panic!("MFT entry size cannot be zero");
        } else {
            bytes.len() / entry_size.get::<byte>()
        };
        Self {
            bytes,
            entry_size,
            index: 0,
            total,
        }
    }
}

impl Iterator for MftRecordIter {
    type Item = MftRecord;
    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.total {
            return None;
        }
        let entry_size_bytes = self.entry_size.get::<byte>();
        let start = self.index * entry_size_bytes;
        let end = start + entry_size_bytes;
        self.index += 1;
        Some(MftRecord::from_bytes_unchecked(
            self.bytes.slice(start..end),
        ))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.total.saturating_sub(self.index);
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for MftRecordIter {}
impl core::iter::FusedIterator for MftRecordIter {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iter_records_yields_zero_copy_records() {
        const ENTRY_SIZE: usize = 1024;
        let mut buf = vec![0u8; ENTRY_SIZE * 2];
        // Write 'FILE' signature for both records
        buf[0..4].copy_from_slice(b"FILE");
        buf[ENTRY_SIZE..ENTRY_SIZE + 4].copy_from_slice(b"FILE");
        let bytes = Bytes::from(buf);
        let mut it = MftRecordIter::new(bytes.clone(), Information::new::<byte>(ENTRY_SIZE));
        let r1 = it.next().expect("first record");
        let r2 = it.next().expect("second record");
        assert!(it.next().is_none());
        assert_eq!(r1.get_signature(), b"FILE");
        assert_eq!(r2.get_signature(), b"FILE");
        assert_eq!(bytes.len(), ENTRY_SIZE * 2);
    }
}
