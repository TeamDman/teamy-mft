#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MftRecordFlags(pub(crate) u16);

impl MftRecordFlags {
    pub const IN_USE: Self = Self(0x0001);
    pub const IS_DIRECTORY: Self = Self(0x0002);

    #[must_use]
    pub fn raw(self) -> u16 {
        self.0
    }

    #[must_use]
    pub fn is_in_use(self) -> bool {
        self.contains(Self::IN_USE)
    }

    #[must_use]
    pub fn is_deleted(self) -> bool {
        !self.is_in_use()
    }

    #[must_use]
    pub fn is_directory(self) -> bool {
        self.contains(Self::IS_DIRECTORY)
    }

    #[must_use]
    pub fn contains(self, other: Self) -> bool {
        (self & other).0 == other.0
    }
}

impl From<u16> for MftRecordFlags {
    fn from(value: u16) -> Self {
        Self(value)
    }
}

impl From<MftRecordFlags> for u16 {
    fn from(value: MftRecordFlags) -> Self {
        value.0
    }
}

impl std::ops::BitOr for MftRecordFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for MftRecordFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl std::ops::BitAnd for MftRecordFlags {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self::Output {
        Self(self.0 & rhs.0)
    }
}

impl std::ops::BitAndAssign for MftRecordFlags {
    fn bitand_assign(&mut self, rhs: Self) {
        self.0 &= rhs.0;
    }
}
