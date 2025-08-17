use std::ops::Deref;

use crate::windows::win_handles::AutoClosingHandle;
use eyre::Context;
use eyre::eyre;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::FSCTL_GET_NTFS_VOLUME_DATA;
use windows::Win32::System::Ioctl::NTFS_VOLUME_DATA_BUFFER;

pub struct NtfsDriveHandle {
    pub handle: AutoClosingHandle,
}
impl NtfsDriveHandle {
    pub fn try_new(drive_handle: AutoClosingHandle) -> eyre::Result<Self> {
        validate_ntfs_filesystem(*drive_handle)?;
        Ok(NtfsDriveHandle {
            handle: drive_handle,
        })
    }
    pub fn new_unchecked(drive_handle: AutoClosingHandle) -> Self {
        NtfsDriveHandle {
            handle: drive_handle,
        }
    }
}
impl TryFrom<AutoClosingHandle> for NtfsDriveHandle {
    type Error = eyre::Report;

    fn try_from(drive_handle: AutoClosingHandle) -> Result<Self, Self::Error> {
        Self::try_new(drive_handle)
    }
}
impl Deref for NtfsDriveHandle {
    type Target = AutoClosingHandle;

    fn deref(&self) -> &Self::Target {
        &self.handle
    }
}

/// Validates that the specified drive is using NTFS filesystem
fn validate_ntfs_filesystem(drive_handle: HANDLE) -> eyre::Result<()> {
    let mut volume_data = NTFS_VOLUME_DATA_BUFFER::default();
    let mut bytes_returned = 0u32;

    let result = unsafe {
        DeviceIoControl(
            drive_handle,
            FSCTL_GET_NTFS_VOLUME_DATA,
            None,
            0,
            Some(&mut volume_data as *mut _ as *mut _),
            size_of::<NTFS_VOLUME_DATA_BUFFER>() as u32,
            Some(&mut bytes_returned),
            None,
        )
    };
    result.wrap_err(eyre!(
        "Drive does not appear to be using NTFS filesystem. FSCTL_GET_NTFS_VOLUME_DATA failed. MFT dumping is only supported on NTFS volumes."
    ))
}
