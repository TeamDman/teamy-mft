use crate::mft::mft_record_number::MftRecordNumber;
use crate::mft::mft_sequence_number::MftSequenceNumber;
use core::fmt;
use core::ops::Add;
use core::ops::Deref;

/// Represents the 8-byte on-disk MFT file reference (`FILE_REFERENCE` / MFT reference).
/// Layout (little-endian):
///   bits 0..=47  : MFT record (entry) number
///   bits 48..=63 : sequence number (stale detection)
#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash, Default)]
pub struct MftRecordReference(u64);

impl MftRecordReference {
    pub const RECORD_NUMBER_MASK: u64 = (1u64 << 48) - 1;

    /// Creates a reference from raw u64 (no validation beyond masking).
    #[must_use]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
    /// Compose from parts, validating record number fits 48 bits (debug only).
    #[must_use]
    pub fn from_parts(record: MftRecordNumber, sequence: MftSequenceNumber) -> Self {
        debug_assert!(
            *record <= Self::RECORD_NUMBER_MASK,
            "record number exceeds 48 bits"
        );
        let raw = (*record & Self::RECORD_NUMBER_MASK) | (u64::from(sequence.get()) << 48);
        Self(raw)
    }
    #[must_use]
    pub fn to_raw(self) -> u64 {
        self.0
    }
    #[must_use]
    pub fn get_record_number(self) -> MftRecordNumber {
        MftRecordNumber::new(self.0 & Self::RECORD_NUMBER_MASK)
    }
    #[must_use]
    pub fn get_sequence_number(self) -> MftSequenceNumber {
        MftSequenceNumber::new((self.0 >> 48) as u16)
    }
}

impl From<u64> for MftRecordReference {
    fn from(value: u64) -> Self {
        Self::from_raw(value)
    }
}

impl Deref for MftRecordReference {
    type Target = u64;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl fmt::Debug for MftRecordReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "MftRecordReference(record={}, sequence={})",
            self.get_record_number(),
            self.get_sequence_number()
        )
    }
}
impl fmt::Display for MftRecordReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}",
            self.get_record_number(),
            self.get_sequence_number()
        )
    }
}

// Addition sugar: RecordNumber + SequenceNumber => RecordReference
impl Add<MftSequenceNumber> for MftRecordNumber {
    type Output = MftRecordReference;
    fn add(self, rhs: MftSequenceNumber) -> Self::Output {
        MftRecordReference::from_parts(self, rhs)
    }
}
impl Add<MftRecordNumber> for MftSequenceNumber {
    type Output = MftRecordReference;
    fn add(self, rhs: MftRecordNumber) -> Self::Output {
        MftRecordReference::from_parts(rhs, self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compose_and_extract() {
        let rec = MftRecordNumber::new(0xFFFF_FFFF_FFFF); // true max 48-bit value
        let seq = MftSequenceNumber::new(0xABCD);
        let r = MftRecordReference::from_parts(rec, seq);
        assert_eq!(*r.get_record_number(), 0xFFFF_FFFF_FFFF);
        assert_eq!(r.get_sequence_number().get(), 0xABCD);
        assert_eq!(r.to_raw(), 0xABCD_FFFFFFFFFFFFu64);
        assert_eq!(r, rec + seq);
    }
    #[test]
    fn add_impls() {
        let rec = MftRecordNumber::new(123);
        let seq = MftSequenceNumber::new(77);
        let a = rec + seq;
        let b = seq + rec;
        assert_eq!(a.to_raw(), b.to_raw());
        assert_eq!(*a.get_record_number(), 123);
        assert_eq!(a.get_sequence_number().get(), 77);
    }
}
