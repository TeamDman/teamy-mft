use std::ops::Deref;
use widestring::U16CString;
use windows::core::PCWSTR;
use windows::core::Param;

/// Prevents `Self` from being dropped before the finish of a FFI call.
#[derive(Debug)]
pub struct PCWSTRGuard {
    string: U16CString,
}
impl PCWSTRGuard {
    pub fn new(string: U16CString) -> Self {
        Self { string }
    }

    /// # Safety
    ///
    /// You must ensure that the `PCWSTRGuard` outlives any usage of the pointer.
    pub unsafe fn as_ptr(&self) -> PCWSTR {
        PCWSTR(self.string.as_ptr())
    }

    pub fn as_wide(&self) -> &[u16] {
        self.string.as_slice()
    }
}
impl Deref for PCWSTRGuard {
    type Target = U16CString;

    fn deref(&self) -> &Self::Target {
        &self.string
    }
}

/// MUST NOT implement this for `PCWSTRGuard` itself, only for `&PCWSTRGuard`,
/// to ensure the data the PCWSTR points to is valid for the lifetime of the parameter.
impl Param<PCWSTR> for &PCWSTRGuard {
    unsafe fn param(self) -> windows::core::ParamValue<PCWSTR> {
        windows::core::ParamValue::Borrowed(PCWSTR(self.string.as_ptr()))
    }
}

/// Included for postfix `.as_ref()` convenience.
impl AsRef<PCWSTRGuard> for PCWSTRGuard {
    fn as_ref(&self) -> &PCWSTRGuard {
        self
    }
}
