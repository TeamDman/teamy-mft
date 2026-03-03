use std::ops::Deref;

/// Typed index into an in-memory MFT record collection.
///
/// This represents positional indexing (0-based) rather than on-disk byte offsets.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash, Default)]
pub struct MftRecordIndex(usize);

impl MftRecordIndex {
    #[must_use]
    pub const fn new(index: usize) -> Self {
        Self(index)
    }

    #[must_use]
    pub const fn get(self) -> usize {
        self.0
    }
}

impl From<usize> for MftRecordIndex {
    fn from(value: usize) -> Self {
        Self(value)
    }
}

impl From<u32> for MftRecordIndex {
    fn from(value: u32) -> Self {
        Self(value as usize)
    }
}

impl From<MftRecordIndex> for usize {
    fn from(value: MftRecordIndex) -> Self {
        value.0
    }
}

impl Deref for MftRecordIndex {
    type Target = usize;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl PartialEq<usize> for MftRecordIndex {
    fn eq(&self, other: &usize) -> bool {
        self.0 == *other
    }
}

impl PartialOrd<usize> for MftRecordIndex {
    fn partial_cmp(&self, other: &usize) -> Option<std::cmp::Ordering> {
        self.0.partial_cmp(other)
    }
}

impl std::ops::AddAssign<usize> for MftRecordIndex {
    fn add_assign(&mut self, rhs: usize) {
        self.0 += rhs;
    }
}

impl std::ops::AddAssign<MftRecordIndex> for MftRecordIndex {
    fn add_assign(&mut self, rhs: MftRecordIndex) {
        self.0 += rhs.0;
    }
}

impl std::ops::Add<usize> for MftRecordIndex {
    type Output = Self;

    fn add(self, rhs: usize) -> Self::Output {
        Self(self.0 + rhs)
    }
}

impl std::ops::Add<MftRecordIndex> for MftRecordIndex {
    type Output = Self;

    fn add(self, rhs: MftRecordIndex) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}
