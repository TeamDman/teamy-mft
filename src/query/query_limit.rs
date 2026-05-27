use crate::domain::FacetSafeNonZeroUsizeOption;
use arbitrary::Arbitrary;
use facet::Facet;
use std::ops::Deref;

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Facet)]
#[repr(transparent)]
pub struct QueryLimit(FacetSafeNonZeroUsizeOption);

impl Deref for QueryLimit {
    type Target = FacetSafeNonZeroUsizeOption;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl QueryLimit {
    #[must_use]
    pub fn get(self) -> Option<usize> {
        self.0.get()
    }

    #[must_use]
    pub fn is_some_and(self, f: impl FnOnce(usize) -> bool) -> bool {
        self.get().is_some_and(f)
    }
}

impl From<usize> for QueryLimit {
    fn from(value: usize) -> Self {
        Self(FacetSafeNonZeroUsizeOption::from(value))
    }
}

impl From<QueryLimit> for usize {
    fn from(value: QueryLimit) -> Self {
        value.0.into()
    }
}

impl From<&QueryLimit> for usize {
    fn from(value: &QueryLimit) -> Self {
        (*value).into()
    }
}

impl std::fmt::Display for QueryLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        usize::from(self).fmt(f)
    }
}

impl<'a> Arbitrary<'a> for QueryLimit {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let value = if bool::arbitrary(u)? {
            usize::try_from(u32::arbitrary(u)?).unwrap_or(usize::MAX - 1) + 1
        } else {
            0
        };
        Ok(Self::from(value))
    }
}
