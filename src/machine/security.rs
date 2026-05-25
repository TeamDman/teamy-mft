#![allow(
    clippy::borrow_as_ptr,
    clippy::cast_ptr_alignment,
    clippy::undocumented_unsafe_blocks,
    reason = "Windows token and SID interop requires raw pointer FFI that is localized in this module"
)]

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Foundation::HLOCAL;
use windows::Win32::Foundation::LocalFree;
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::GetTokenInformation;
use windows::Win32::Security::TOKEN_QUERY;
use windows::Win32::Security::TOKEN_USER;
use windows::Win32::Security::TokenUser;
use windows::Win32::System::Threading::GetCurrentProcess;
use windows::Win32::System::Threading::OpenProcessToken;
use windows::core::PCWSTR;
use windows::core::PWSTR;

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
    take_ownership(path)?;
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
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let failed_processing_count = parse_icacls_failed_processing_count(&stdout)
        .max(parse_icacls_failed_processing_count(&stderr));
    if !output.status.success() || failed_processing_count > 0 {
        eyre::bail!("icacls failed for {}: {}{}", path.display(), stdout, stderr);
    }
    Ok(())
}

fn parse_icacls_failed_processing_count(output: &str) -> u64 {
    output
        .lines()
        .filter_map(|line| line.split_once("Failed processing "))
        .filter_map(|(_, remainder)| remainder.split_whitespace().next())
        .find_map(|count| count.parse::<u64>().ok())
        .unwrap_or(0)
}

/// # Errors
///
/// Returns an error if ownership of the path cannot be reassigned to administrators.
pub fn take_ownership(path: &Path) -> eyre::Result<()> {
    let mut command = std::process::Command::new("takeown.exe");
    command.arg("/F").arg(path).arg("/A");
    if path.is_dir() {
        command.arg("/R").arg("/D").arg("Y");
    }
    let output = command.output()?;
    if !output.status.success() {
        eyre::bail!(
            "takeown failed for {}: {}{}",
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

#[must_use]
pub fn wide_pcwstr(buffer: &[u16]) -> PCWSTR {
    PCWSTR(buffer.as_ptr())
}
