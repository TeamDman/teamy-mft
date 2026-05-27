use crate::windows_utils::string::EasyPCWSTR;
use eyre::Context;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::CreateFileW;
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
use windows::Win32::Storage::FileSystem::FILE_GENERIC_READ;
use windows::Win32::Storage::FileSystem::FILE_SHARE_DELETE;
use windows::Win32::Storage::FileSystem::FILE_SHARE_READ;
use windows::Win32::Storage::FileSystem::FILE_SHARE_WRITE;
use windows::Win32::Storage::FileSystem::OPEN_EXISTING;
use windows::core::Owned;

pub fn get_read_only_drive_handle(drive_letter: char) -> eyre::Result<Owned<HANDLE>> {
    let drive_path = format!("\\\\.\\{drive_letter}:");
    let raw_handle = unsafe {
        CreateFileW(
            drive_path.easy_pcwstr()?.as_ref(),
            FILE_GENERIC_READ.0,
            windows::Win32::Storage::FileSystem::FILE_SHARE_MODE(
                FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0,
            ),
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    };
    let handle = raw_handle.wrap_err(format!(
        "Failed to open volume handle for {drive_letter:?}, did you forget to elevate?"
    ))?;

    Ok(unsafe { Owned::new(handle) })
}
