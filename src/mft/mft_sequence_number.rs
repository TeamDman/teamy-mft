use core::fmt;
use core::ops::Deref;

/// Wrapper type for the 16-bit NTFS MFT sequence number (used for stale reference detection).
#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash, Default)]
pub struct MftSequenceNumber(pub u16);

impl MftSequenceNumber {
    #[must_use]
    pub fn new(value: u16) -> Self {
        Self(value)
    }
    #[must_use]
    pub fn get(self) -> u16 {
        self.0
    }
}

impl From<u16> for MftSequenceNumber {
    fn from(value: u16) -> Self {
        Self(value)
    }
}

impl Deref for MftSequenceNumber {
    type Target = u16;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl fmt::Debug for MftSequenceNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MftSequenceNumber({})", self.0)
    }
}
impl fmt::Display for MftSequenceNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn basic_new() {
        let s = MftSequenceNumber::new(42);
        assert_eq!(s.get(), 42);
    }
}
