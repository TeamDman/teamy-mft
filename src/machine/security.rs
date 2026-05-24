use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::Win32::Foundation::LocalFree;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::{PCWSTR, PWSTR};

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
}

#[must_use]
pub fn encode_wide(value: &str) -> Vec<u16> {
    std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// # Errors
///
/// Returns an error if the current process token cannot be read.
pub fn current_user_sid_string() -> eyre::Result<String> {
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }?;
    let _owned_token = OwnedHandle(token);

    let mut required_len = 0u32;
    let _ = unsafe { GetTokenInformation(token, TokenUser, None, 0, &mut required_len) };
    if required_len == 0 {
        eyre::bail!("Failed determining current token information length");
    }

    let mut buffer = vec![0u8; required_len as usize];
    unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            Some(buffer.as_mut_ptr().cast::<c_void>()),
            required_len,
            &mut required_len,
        )
    }?;

    let token_user = unsafe { &*(buffer.as_ptr().cast::<TOKEN_USER>()) };
    let mut sid_ptr = PWSTR::null();
    unsafe { ConvertSidToStringSidW(token_user.User.Sid, &mut sid_ptr) }?;
    let sid = unsafe { sid_ptr.to_string() }?;
    let _ = unsafe { LocalFree(Some(HLOCAL(sid_ptr.0.cast()))) };
    Ok(sid)
}

/// # Errors
///
/// Returns an error if the directory ACL cannot be restricted with `icacls`.
pub fn restrict_path_to_owner(path: &Path, owner_sid: &str) -> eyre::Result<()> {
    let owner = format!("*{owner_sid}:(OI)(CI)F");
    let system = "*S-1-5-18:(OI)(CI)F";
    let admins = "*S-1-5-32-544:(OI)(CI)F";
    let output = std::process::Command::new("icacls.exe")
        .arg(path)
        .arg("/inheritance:r")
        .arg("/grant:r")
        .arg(owner)
        .arg("/grant:r")
        .arg(system)
        .arg("/grant:r")
        .arg(admins)
        .arg("/T")
        .output()?;
    if !output.status.success() {
        eyre::bail!(
            "icacls failed for {}: {}{}",
            path.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

#[must_use]
pub fn service_sddl(owner_sid: &str) -> String {
    format!(
        "D:(A;;CCDCLCSWRPWPDTLOCRSDRCWDWO;;;SY)(A;;CCDCLCSWRPWPDTLOCRSDRCWDWO;;;BA)(A;;RPLOLC;;;{owner_sid})"
    )
}

#[must_use]
pub fn named_pipe_sddl(owner_sid: &str) -> String {
    format!("D:(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;{owner_sid})")
}

pub fn wide_pcwstr(buffer: &[u16]) -> PCWSTR {
    PCWSTR(buffer.as_ptr())
}
