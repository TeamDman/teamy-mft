use crate::windows_utils::string::pcwstr_guard::PCWSTRGuard;
use eyre::eyre;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::path::Path;
use std::path::PathBuf;
use widestring::U16CString;
use windows::core::PCWSTR;
use windows::core::PWSTR;

/// Conversion to `PCWSTRGuard` from various string types for easy FFI usage.
pub trait EasyPCWSTR {
    fn easy_pcwstr(self) -> eyre::Result<PCWSTRGuard>;
}

impl EasyPCWSTR for U16CString {
    fn easy_pcwstr(self) -> eyre::Result<PCWSTRGuard> {
        Ok(PCWSTRGuard::new(self))
    }
}

impl EasyPCWSTR for &str {
    fn easy_pcwstr(self) -> eyre::Result<PCWSTRGuard> {
        Ok(PCWSTRGuard::new(U16CString::from_str(self).map_err(
            |_| eyre!("Failed to convert `&str` to U16CString: {}", self),
        )?))
    }
}

impl EasyPCWSTR for &OsString {
    fn easy_pcwstr(self) -> eyre::Result<PCWSTRGuard> {
        Ok(PCWSTRGuard::new(U16CString::from_os_str_truncate(self)))
    }
}

impl EasyPCWSTR for &OsStr {
    fn easy_pcwstr(self) -> eyre::Result<PCWSTRGuard> {
        Ok(PCWSTRGuard::new(U16CString::from_os_str_truncate(self)))
    }
}

impl EasyPCWSTR for &PathBuf {
    fn easy_pcwstr(self) -> eyre::Result<PCWSTRGuard> {
        Ok(PCWSTRGuard::new(U16CString::from_os_str_truncate(
            self.as_os_str(),
        )))
    }
}

impl EasyPCWSTR for &Path {
    fn easy_pcwstr(self) -> eyre::Result<PCWSTRGuard> {
        Ok(PCWSTRGuard::new(U16CString::from_os_str_truncate(
            self.as_os_str(),
        )))
    }
}

impl EasyPCWSTR for PWSTR {
    fn easy_pcwstr(self) -> eyre::Result<PCWSTRGuard> {
        // SAFETY: PWSTR is expected to point to a valid null-terminated wide string
        let u16cstring = unsafe { U16CString::from_ptr_str(self.as_ptr()) };
        Ok(PCWSTRGuard::new(u16cstring))
    }
}

impl EasyPCWSTR for PCWSTR {
    fn easy_pcwstr(self) -> eyre::Result<PCWSTRGuard> {
        // SAFETY: PCWSTR is expected to point to a valid null-terminated wide string
        let u16cstring = unsafe { U16CString::from_ptr_str(self.as_ptr()) };
        Ok(PCWSTRGuard::new(u16cstring))
    }
}

#[cfg(test)]
mod test {
    use super::EasyPCWSTR;
    use std::ffi::OsString;
    use std::path::Path;
    use std::path::PathBuf;
    use widestring::U16CString;

    #[test]
    fn it_works() -> eyre::Result<()> {
        "Hello, World!".easy_pcwstr()?;
        OsString::from("asd").easy_pcwstr()?;
        "asd".to_string().easy_pcwstr()?;
        PathBuf::from("asd").easy_pcwstr()?;
        Path::new("asd").easy_pcwstr()?;
        U16CString::from_str("asd")?.easy_pcwstr()?;
        Ok(())
    }
}
