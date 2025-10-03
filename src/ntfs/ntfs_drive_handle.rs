use eyre::Context;
use eyre::eyre;
use std::mem::size_of;
use std::ops::Deref;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::FSCTL_GET_NTFS_VOLUME_DATA;
use windows::Win32::System::Ioctl::NTFS_VOLUME_DATA_BUFFER;
use windows::Win32::System::Ioctl::VOLUME_DISK_EXTENTS;
use windows::core::Owned;

const IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS: u32 = 0x00560000;

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

/// Get the disk extents for the volume associated with the drive letter
pub fn get_volume_disk_extents(drive_letter: char) -> eyre::Result<VOLUME_DISK_EXTENTS> {
    use teamy_windows::handle::get_read_only_drive_handle;

    let handle = get_read_only_drive_handle(drive_letter)
        .wrap_err_with(|| format!("Failed to open handle to drive {drive_letter}"))?;

    let mut extents = VOLUME_DISK_EXTENTS::default();
    let mut bytes_returned = 0u32;

    unsafe {
        DeviceIoControl(
            *handle,
            IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
            None,
            0,
            Some(&mut extents as *mut _ as *mut std::ffi::c_void),
            size_of::<VOLUME_DISK_EXTENTS>() as u32,
            Some(&mut bytes_returned),
            None,
        )
        .wrap_err("DeviceIoControl for IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS failed")?;
    }

    if extents.NumberOfDiskExtents != 1 {
        eyre::bail!("Volume spans multiple disks or has complex extents, not supported");
    }

    Ok(extents)
}
