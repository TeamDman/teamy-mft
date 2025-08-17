use crate::windows::win_strings::EasyPCWSTR;
use eyre::Context;
use std::ops::Deref;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::CreateFileW;
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
use windows::Win32::Storage::FileSystem::FILE_GENERIC_READ;
use windows::Win32::Storage::FileSystem::FILE_SHARE_DELETE;
use windows::Win32::Storage::FileSystem::FILE_SHARE_READ;
use windows::Win32::Storage::FileSystem::FILE_SHARE_WRITE;
use windows::Win32::Storage::FileSystem::OPEN_EXISTING;

/// Auto-closing handle wrapper
pub struct AutoClosingHandle(HANDLE);
impl Deref for AutoClosingHandle {
    type Target = HANDLE;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl Drop for AutoClosingHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
impl AutoClosingHandle {
    pub fn new(handle: HANDLE) -> Self {
        Self(handle)
    }
}
impl From<HANDLE> for AutoClosingHandle {
    fn from(handle: HANDLE) -> Self {
        AutoClosingHandle(handle)
    }
}

/// Opens a handle to the specified drive.
pub fn get_drive_handle(drive_letter: char) -> eyre::Result<AutoClosingHandle> {
    let drive_path = format!("\\\\.\\{drive_letter}:");
    let handle = unsafe {
        CreateFileW(
            drive_path.easy_pcwstr()?.as_ref(),
            FILE_GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
        .wrap_err(format!(
            "Failed to open volume handle for {drive_letter:?}, did you forget to elevate?"
        ))?
    };

    Ok(AutoClosingHandle(handle))
}
