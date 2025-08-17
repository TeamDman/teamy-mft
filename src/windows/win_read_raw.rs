use eyre::Context;
use tracing::info;
use windows::Win32::Foundation::LUID;
use windows::Win32::Security::AdjustTokenPrivileges;
use windows::Win32::Security::LookupPrivilegeValueW;
use windows::Win32::Security::SE_BACKUP_NAME;
use windows::Win32::Security::SE_PRIVILEGE_ENABLED;
use windows::Win32::Security::SE_RESTORE_NAME;
use windows::Win32::Security::SE_SECURITY_NAME;
use windows::Win32::Security::TOKEN_ADJUST_PRIVILEGES;
use windows::Win32::Security::TOKEN_PRIVILEGES;
use windows::Win32::Security::TOKEN_QUERY;
use windows::Win32::System::Threading::GetCurrentProcess;
use windows::Win32::System::Threading::OpenProcessToken;

/// Enables backup and security privileges for the current process
pub fn enable_backup_privileges() -> eyre::Result<()> {
    use std::mem::size_of;

    unsafe {
        // Get current process token
        let mut token = windows::Win32::Foundation::HANDLE::default();
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        )
        .wrap_err_with(|| "Failed to open process token")?;

        // Enable multiple privileges that might be needed
        let privileges_to_enable = [SE_BACKUP_NAME, SE_RESTORE_NAME, SE_SECURITY_NAME];

        for privilege_name in &privileges_to_enable {
            // Look up the privilege LUID
            let mut luid = LUID::default();
            if LookupPrivilegeValueW(None, *privilege_name, &mut luid).is_ok() {
                // Set up the privilege structure
                let privileges = TOKEN_PRIVILEGES {
                    PrivilegeCount: 1,
                    Privileges: [windows::Win32::Security::LUID_AND_ATTRIBUTES {
                        Luid: luid,
                        Attributes: SE_PRIVILEGE_ENABLED,
                    }],
                };

                // Adjust token privileges
                let _ = AdjustTokenPrivileges(
                    token,
                    false,
                    Some(&privileges),
                    size_of::<TOKEN_PRIVILEGES>() as u32,
                    None,
                    None,
                );
            }
        }

        // Close token handle
        windows::Win32::Foundation::CloseHandle(token)
            .wrap_err_with(|| "Failed to close token handle")?;

        info!("Successfully enabled backup privileges");
        Ok(())
    }
}
