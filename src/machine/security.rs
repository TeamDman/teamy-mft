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
use windows::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
use windows::Win32::Security::Authorization::ConvertStringSidToSidW;
use windows::Win32::Security::Authorization::SDDL_REVISION_1;
use windows::Win32::Security::GetTokenInformation;
use windows::Win32::Security::LookupAccountSidW;
use windows::Win32::Security::PSECURITY_DESCRIPTOR;
use windows::Win32::Security::PSID;
use windows::Win32::Security::SECURITY_ATTRIBUTES;
use windows::Win32::Security::SID_NAME_USE;
use windows::Win32::Security::TOKEN_QUERY;
use windows::Win32::Security::TOKEN_USER;
use windows::Win32::Security::TokenUser;
use windows::Win32::System::Threading::GetCurrentProcess;
use windows::Win32::System::Threading::OpenProcessToken;
use windows::core::PCWSTR;
use windows::core::PWSTR;

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "Protection status is reported as explicit flat CLI fields"
)]
pub struct PathProtectionStatus {
    pub path_exists: bool,
    pub owner_sid_grant_present: bool,
    pub system_grant_present: bool,
    pub administrators_grant_present: bool,
    pub broad_read_grant_present: bool,
    pub inheritance_disabled: bool,
    pub raw_acl: String,
}

impl PathProtectionStatus {
    #[must_use]
    pub fn protection_enabled(&self) -> bool {
        self.path_exists
            && self.owner_sid_grant_present
            && self.system_grant_present
            && self.administrators_grant_present
            && self.inheritance_disabled
            && !self.broad_read_grant_present
    }
}

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

/// # Errors
///
/// Returns an error if the development read ACL cannot be applied with `icacls`.
pub fn allow_development_reads(path: &Path) -> eyre::Result<()> {
    take_ownership(path)?;
    grant_development_read_to_path(path)?;
    Ok(())
}

fn grant_development_read_to_path(path: &Path) -> eyre::Result<()> {
    let path_is_dir = path.is_dir();
    let users = if path_is_dir {
        "*S-1-5-32-545:(OI)(CI)RX"
    } else {
        "*S-1-5-32-545:RX"
    };
    let everyone = if path_is_dir {
        "*S-1-1-0:(OI)(CI)RX"
    } else {
        "*S-1-1-0:RX"
    };
    let mut command = std::process::Command::new("icacls.exe");
    command
        .arg(path)
        .arg("/grant:r")
        .arg(users)
        .arg("/grant:r")
        .arg(everyone);
    if path_is_dir {
        command.arg("/T").arg("/C");
    }
    let output = command.output()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let failed_processing_count = parse_icacls_failed_processing_count(&stdout)
        .max(parse_icacls_failed_processing_count(&stderr));
    if !output.status.success() || failed_processing_count > 0 {
        eyre::bail!(
            "icacls failed while enabling development reads for {}: {}{}",
            path.display(),
            stdout,
            stderr
        );
    }
    Ok(())
}

/// # Errors
///
/// Returns an error if the ACL cannot be queried with `icacls`.
pub fn query_path_protection_status(
    path: &Path,
    owner_sid: &str,
) -> eyre::Result<PathProtectionStatus> {
    if !path.exists() {
        return Ok(PathProtectionStatus {
            path_exists: false,
            owner_sid_grant_present: false,
            system_grant_present: false,
            administrators_grant_present: false,
            broad_read_grant_present: false,
            inheritance_disabled: false,
            raw_acl: String::new(),
        });
    }

    let output = std::process::Command::new("icacls.exe")
        .arg(path)
        .output()?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        eyre::bail!(
            "icacls failed while reading ACL for {}: {}{}",
            path.display(),
            stdout,
            stderr
        );
    }

    let owner = owner_sid.to_ascii_uppercase();
    let owner_account = account_name_from_sid_string(owner_sid)
        .ok()
        .map(|account| account.to_ascii_uppercase());
    let acl_lines = stdout
        .lines()
        .map(str::to_ascii_uppercase)
        .collect::<Vec<_>>();
    Ok(PathProtectionStatus {
        path_exists: true,
        owner_sid_grant_present: acl_contains_principal(&acl_lines, &owner)
            || owner_account
                .as_ref()
                .is_some_and(|account| acl_contains_principal(&acl_lines, account)),
        system_grant_present: acl_contains_principal(&acl_lines, "NT AUTHORITY\\SYSTEM")
            || acl_contains_principal(&acl_lines, "S-1-5-18"),
        administrators_grant_present: acl_contains_principal(&acl_lines, "BUILTIN\\ADMINISTRATORS")
            || acl_contains_principal(&acl_lines, "S-1-5-32-544"),
        broad_read_grant_present: acl_contains_principal(&acl_lines, "BUILTIN\\USERS")
            || acl_contains_principal(&acl_lines, "S-1-5-32-545")
            || acl_contains_principal(&acl_lines, "EVERYONE")
            || acl_contains_principal(&acl_lines, "S-1-1-0"),
        inheritance_disabled: !acl_lines
            .iter()
            .any(|line| acl_line_has_access_mask(line) && line.contains("(I)")),
        raw_acl: stdout,
    })
}

fn acl_contains_principal(acl_lines: &[String], principal: &str) -> bool {
    let principal = principal.trim_start_matches('*');
    let sid_principal = format!("*{principal}");
    acl_lines.iter().any(|line| {
        acl_line_has_principal(line, principal) || acl_line_has_principal(line, &sid_principal)
    })
}

fn acl_line_has_principal(line: &str, principal: &str) -> bool {
    let Some(principal_end) = line.find(":(") else {
        return false;
    };
    let before_access_mask = line[..principal_end].trim_end();
    let Some(prefix) = before_access_mask.strip_suffix(principal) else {
        return false;
    };
    prefix.chars().last().is_none_or(char::is_whitespace)
}

fn acl_line_has_access_mask(line: &str) -> bool {
    line.contains(":(")
}

pub fn warn_if_path_protection_disabled(path: &Path, status: &PathProtectionStatus) {
    if status.protection_enabled() {
        return;
    }
    tracing::warn!(
        cache_root = %path.display(),
        broad_read_grant_present = status.broad_read_grant_present,
        "Machine cache protection is disabled; this should only be expected while developing teamy-mft locally. Run `teamy-mft protection enable` before normal use."
    );
}

pub fn print_path_protection_status(status: &PathProtectionStatus) {
    println!("machine-protection-enabled={}", status.protection_enabled());
    println!("machine-protection-path-exists={}", status.path_exists);
    println!(
        "machine-protection-owner-grant-present={}",
        status.owner_sid_grant_present
    );
    println!(
        "machine-protection-system-grant-present={}",
        status.system_grant_present
    );
    println!(
        "machine-protection-administrators-grant-present={}",
        status.administrators_grant_present
    );
    println!(
        "machine-protection-broad-read-grant-present={}",
        status.broad_read_grant_present
    );
    println!(
        "machine-protection-inheritance-disabled={}",
        status.inheritance_disabled
    );
}

fn account_name_from_sid_string(sid: &str) -> eyre::Result<String> {
    let wide_sid = encode_wide(sid);
    let mut sid_ptr = PSID::default();
    unsafe { ConvertStringSidToSidW(PCWSTR(wide_sid.as_ptr()), &mut sid_ptr) }?;
    let _sid_guard = SidLocalFreeGuard(sid_ptr);

    let mut name_len = 0u32;
    let mut domain_len = 0u32;
    let mut sid_name_use = SID_NAME_USE(0);
    let _ = unsafe {
        LookupAccountSidW(
            PCWSTR::null(),
            sid_ptr,
            None,
            &mut name_len,
            None,
            &mut domain_len,
            &mut sid_name_use,
        )
    };
    if name_len == 0 {
        eyre::bail!("Failed determining account name length for SID {sid}");
    }

    let mut name = vec![0u16; name_len as usize];
    let mut domain = vec![0u16; domain_len.max(1) as usize];
    let domain_ptr = if domain_len == 0 {
        None
    } else {
        Some(PWSTR(domain.as_mut_ptr()))
    };
    unsafe {
        LookupAccountSidW(
            PCWSTR::null(),
            sid_ptr,
            Some(PWSTR(name.as_mut_ptr())),
            &mut name_len,
            domain_ptr,
            &mut domain_len,
            &mut sid_name_use,
        )
    }?;

    name.truncate(name_len as usize);
    domain.truncate(domain_len as usize);
    let name = String::from_utf16_lossy(&name);
    let domain = String::from_utf16_lossy(&domain);
    if domain.is_empty() {
        Ok(name)
    } else {
        Ok(format!("{domain}\\{name}"))
    }
}

struct SidLocalFreeGuard(PSID);

impl Drop for SidLocalFreeGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            let _ = unsafe { LocalFree(Some(HLOCAL(self.0.0.cast()))) };
        }
    }
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

#[derive(Debug)]
pub struct OwnedSecurityAttributes {
    attributes: SECURITY_ATTRIBUTES,
    descriptor: PSECURITY_DESCRIPTOR,
}

impl OwnedSecurityAttributes {
    #[must_use]
    pub fn as_mut_ptr(&mut self) -> *mut c_void {
        std::ptr::from_mut::<SECURITY_ATTRIBUTES>(&mut self.attributes).cast::<c_void>()
    }
}

impl Drop for OwnedSecurityAttributes {
    fn drop(&mut self) {
        if !self.descriptor.0.is_null() {
            let _ = unsafe { LocalFree(Some(HLOCAL(self.descriptor.0.cast()))) };
        }
    }
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

/// # Errors
///
/// Returns an error if the named pipe security descriptor cannot be created from SDDL.
pub fn named_pipe_security_attributes(owner_sid: &str) -> eyre::Result<OwnedSecurityAttributes> {
    let sddl = named_pipe_sddl(owner_sid);
    let wide = encode_wide(&sddl);
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(wide.as_ptr()),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
    }?;

    #[allow(
        clippy::cast_possible_truncation,
        reason = "SECURITY_ATTRIBUTES length fits in u32 on supported Windows targets"
    )]
    let length = std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32;

    Ok(OwnedSecurityAttributes {
        attributes: SECURITY_ATTRIBUTES {
            nLength: length,
            lpSecurityDescriptor: descriptor.0.cast::<c_void>(),
            bInheritHandle: false.into(),
        },
        descriptor,
    })
}

#[must_use]
pub fn wide_pcwstr(buffer: &[u16]) -> PCWSTR {
    PCWSTR(buffer.as_ptr())
}
