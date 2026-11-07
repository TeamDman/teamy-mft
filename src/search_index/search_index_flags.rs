/// Flags reserved by the current search-index format.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
#[repr(u16)]
pub enum SearchIndexFlags {
    Reserved = 0,
}

impl TryFrom<u16> for SearchIndexFlags {
    type Error = eyre::Report;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Reserved),
            _ => eyre::bail!("Unsupported search index flags: {value:#06x}"),
        }
    }
}

impl From<SearchIndexFlags> for u16 {
    fn from(value: SearchIndexFlags) -> Self {
        value as Self
    }
}

#[cfg(test)]
mod tests {
    use super::SearchIndexFlags;

    #[test]
    fn reserved_flags_accept_only_zero() -> eyre::Result<()> {
        assert_eq!(SearchIndexFlags::try_from(0)?, SearchIndexFlags::Reserved);
        assert_eq!(u16::from(SearchIndexFlags::Reserved), 0);
        for value in [1, 2, 0x8000, u16::MAX] {
            assert!(SearchIndexFlags::try_from(value).is_err());
        }
        Ok(())
    }
}
