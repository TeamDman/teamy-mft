use std::ops::Deref;

/// Wrapper for an NTFS MFT record number (a.k.a. file reference without the sequence part).
#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct MftRecordNumber {
    pub record_number: u64,
}

impl MftRecordNumber {
    // ---------------------------------------------------------------------
    // Reserved system file record numbers (0–15) as defined by NTFS.
    // Reference: Windows Internals / NTFS documentation.
    // These records are guaranteed to exist (though some may be empty on a
    // given volume) and the first 16 slots are reserved for them.
    // ---------------------------------------------------------------------
    /// 0 – $MFT (Master File Table itself)
    pub const DOLLAR_MFT: MftRecordNumber = MftRecordNumber { record_number: 0 };
    /// 1 – $MFTMirr (mirror of first 4 records of $MFT)
    pub const DOLLAR_MFT_MIRR: MftRecordNumber = MftRecordNumber { record_number: 1 };
    /// 2 – $LogFile (transaction log)
    pub const DOLLAR_LOG_FILE: MftRecordNumber = MftRecordNumber { record_number: 2 };
    /// 3 – $Volume (volume information)
    pub const DOLLAR_VOLUME: MftRecordNumber = MftRecordNumber { record_number: 3 };
    /// 4 – $AttrDef (attribute definitions)
    pub const DOLLAR_ATTR_DEF: MftRecordNumber = MftRecordNumber { record_number: 4 };
    /// 5 – Root directory (".")
    pub const MFT_ROOT: MftRecordNumber = MftRecordNumber { record_number: 5 };
    /// 6 – $Bitmap (cluster allocation bitmap)
    pub const DOLLAR_BITMAP: MftRecordNumber = MftRecordNumber { record_number: 6 };
    /// 7 – $Boot (boot sector / volume boot record copy)
    pub const DOLLAR_BOOT: MftRecordNumber = MftRecordNumber { record_number: 7 };
    /// 8 – $BadClus (lists bad clusters)
    pub const DOLLAR_BAD_CLUS: MftRecordNumber = MftRecordNumber { record_number: 8 };
    /// 9 – $Secure (security descriptors store)
    pub const DOLLAR_SECURE: MftRecordNumber = MftRecordNumber { record_number: 9 };
    /// 10 – $UpCase (uppercase mapping table)
    pub const DOLLAR_UP_CASE: MftRecordNumber = MftRecordNumber { record_number: 10 };
    /// 11 – $Extend (directory containing extend metadata files)
    pub const DOLLAR_EXTEND: MftRecordNumber = MftRecordNumber { record_number: 11 };
    /// 12 – $Quota (user quota information)
    pub const DOLLAR_QUOTA: MftRecordNumber = MftRecordNumber { record_number: 12 };
    /// 13 – $ObjId (object IDs)
    pub const DOLLAR_OBJ_ID: MftRecordNumber = MftRecordNumber { record_number: 13 };
    /// 14 – $Reparse (reparse point data)
    pub const DOLLAR_REPARSE: MftRecordNumber = MftRecordNumber { record_number: 14 };
    /// 15 – $UsnJrnl (Update Sequence Number change journal)
    pub const DOLLAR_USN_JRNL: MftRecordNumber = MftRecordNumber { record_number: 15 };

    /// Returns true if this record number is within the reserved system file range (0–15 inclusive).
    pub const fn is_reserved(self) -> bool {
        self.record_number < 16
    }
}

impl From<u64> for MftRecordNumber {
    fn from(record_number: u64) -> Self {
        MftRecordNumber { record_number }
    }
}

impl Deref for MftRecordNumber {
    type Target = u64;
    fn deref(&self) -> &Self::Target { &self.record_number }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_constants_are_correct() {
        assert_eq!(*MftRecordNumber::DOLLAR_MFT, 0);
        assert_eq!(*MftRecordNumber::DOLLAR_MFT_MIRR, 1);
        assert_eq!(*MftRecordNumber::DOLLAR_LOG_FILE, 2);
        assert_eq!(*MftRecordNumber::DOLLAR_VOLUME, 3);
        assert_eq!(*MftRecordNumber::DOLLAR_ATTR_DEF, 4);
        assert_eq!(*MftRecordNumber::MFT_ROOT, 5);
        assert_eq!(*MftRecordNumber::DOLLAR_BITMAP, 6);
        assert_eq!(*MftRecordNumber::DOLLAR_BOOT, 7);
        assert_eq!(*MftRecordNumber::DOLLAR_BAD_CLUS, 8);
        assert_eq!(*MftRecordNumber::DOLLAR_SECURE, 9);
        assert_eq!(*MftRecordNumber::DOLLAR_UP_CASE, 10);
        assert_eq!(*MftRecordNumber::DOLLAR_EXTEND, 11);
        assert_eq!(*MftRecordNumber::DOLLAR_QUOTA, 12);
        assert_eq!(*MftRecordNumber::DOLLAR_OBJ_ID, 13);
        assert_eq!(*MftRecordNumber::DOLLAR_REPARSE, 14);
        assert_eq!(*MftRecordNumber::DOLLAR_USN_JRNL, 15);
        assert!(MftRecordNumber::DOLLAR_QUOTA.is_reserved());
        assert!(!MftRecordNumber::from(16).is_reserved());
    }
}