use eyre::Context;
use eyre::eyre;
use windows::core::Owned;
use std::ops::Deref;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::FSCTL_GET_NTFS_VOLUME_DATA;
use windows::Win32::System::Ioctl::NTFS_VOLUME_DATA_BUFFER;

pub struct NtfsDriveHandle {
    pub handle: Owned<HANDLE>,
}
impl NtfsDriveHandle {
    pub fn try_new(drive_handle: Owned<HANDLE>) -> eyre::Result<Self> {
        validate_ntfs_filesystem(*drive_handle)?;
        Ok(NtfsDriveHandle {
            handle: drive_handle,
        })
    }
    pub fn new_unchecked(drive_handle: Owned<HANDLE>) -> Self {
        NtfsDriveHandle {
            handle: drive_handle,
        }
    }
}
impl TryFrom<Owned<HANDLE>> for NtfsDriveHandle {
    type Error = eyre::Report;

    fn try_from(drive_handle: Owned<HANDLE>) -> Result<Self, Self::Error> {
        Self::try_new(drive_handle)
    }
}
impl Deref for NtfsDriveHandle {
    type Target = Owned<HANDLE>;

    fn deref(&self) -> &Self::Target {
        &self.handle
    }
}
impl AsRef<HANDLE> for NtfsDriveHandle {
    fn as_ref(&self) -> &HANDLE {
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
