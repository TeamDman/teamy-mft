use crate::mft::mft_record::MftRecord;
use crate::mft::mft_record_size::MftRecordSize;
use bytes::Bytes;
use uom::si::information::byte;

/// Zero-copy iterator over MFT records stored contiguously in a `Bytes` buffer.
#[derive(Debug)]
pub struct MftRecordIter {
    bytes: Bytes,
    entry_size: MftRecordSize,
    index: usize,
    total_record_count: usize,
}

impl MftRecordIter {
    /// # Panics
    /// Panics if `entry_size` is zero.
    pub fn new(bytes: Bytes, entry_size: MftRecordSize) -> Self {
        let entry_size_bytes = entry_size.get::<byte>();
        let total_record_count = bytes.len() / entry_size_bytes;
        Self {
            bytes,
            entry_size,
            index: 0,
            total_record_count,
        }
    }
}

impl Iterator for MftRecordIter {
    type Item = MftRecord;
    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.total_record_count {
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
        let remaining = self.total_record_count.saturating_sub(self.index);
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for MftRecordIter {}
impl core::iter::FusedIterator for MftRecordIter {}

#[cfg(test)]
mod tests {
    use super::*;
    use uom::si::usize::Information;

    #[test]
    fn iter_records_yields_zero_copy_records() {
        const ENTRY_SIZE: usize = 1024;
        let mut buf = vec![0u8; ENTRY_SIZE * 2];
        // Write 'FILE' signature for both records
        buf[0..4].copy_from_slice(b"FILE");
        buf[ENTRY_SIZE..ENTRY_SIZE + 4].copy_from_slice(b"FILE");
        let bytes = Bytes::from(buf);
        let mut it = MftRecordIter::new(
            bytes.clone(),
            MftRecordSize::new(Information::new::<byte>(ENTRY_SIZE)).expect("valid record size"),
        );
        let r1 = it.next().expect("first record");
        let r2 = it.next().expect("second record");
        assert!(it.next().is_none());
        assert_eq!(r1.get_signature(), b"FILE");
        assert_eq!(r2.get_signature(), b"FILE");
        assert_eq!(bytes.len(), ENTRY_SIZE * 2);
    }
}
