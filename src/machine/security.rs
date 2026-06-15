#![allow(
    clippy::borrow_as_ptr,
    clippy::cast_ptr_alignment,
    clippy::undocumented_unsafe_blocks,
    reason = "Windows token and SID interop requires raw pointer FFI that is localized in this module"
)]

use crate::windows_utils::string::EasyPCWSTR;
use eyre::WrapErr;
use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::MetadataExt;
use std::path::Path;
use std::path::PathBuf;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Foundation::HLOCAL;
use windows::Win32::Foundation::LocalFree;
use windows::Win32::Foundation::WIN32_ERROR;
use windows::Win32::Security::ACCESS_ALLOWED_ACE;
use windows::Win32::Security::ACE_FLAGS;
use windows::Win32::Security::ACL;
use windows::Win32::Security::ACL_SIZE_INFORMATION;
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
use windows::Win32::Security::Authorization::ConvertStringSidToSidW;
use windows::Win32::Security::Authorization::EXPLICIT_ACCESS_W;
use windows::Win32::Security::Authorization::GetNamedSecurityInfoW;
use windows::Win32::Security::Authorization::SDDL_REVISION_1;
use windows::Win32::Security::Authorization::SE_FILE_OBJECT;
use windows::Win32::Security::Authorization::SET_ACCESS;
use windows::Win32::Security::Authorization::SetEntriesInAclW;
use windows::Win32::Security::Authorization::SetNamedSecurityInfoW;
use windows::Win32::Security::Authorization::TRUSTEE_IS_SID;
use windows::Win32::Security::Authorization::TRUSTEE_IS_UNKNOWN;
use windows::Win32::Security::Authorization::TRUSTEE_W;
use windows::Win32::Security::CONTAINER_INHERIT_ACE;
use windows::Win32::Security::CreateWellKnownSid;
use windows::Win32::Security::DACL_SECURITY_INFORMATION;
use windows::Win32::Security::EqualSid;
use windows::Win32::Security::GetAce;
use windows::Win32::Security::GetAclInformation;
use windows::Win32::Security::GetTokenInformation;
use windows::Win32::Security::INHERITED_ACE;
use windows::Win32::Security::OBJECT_INHERIT_ACE;
use windows::Win32::Security::OWNER_SECURITY_INFORMATION;
use windows::Win32::Security::PROTECTED_DACL_SECURITY_INFORMATION;
use windows::Win32::Security::PSECURITY_DESCRIPTOR;
use windows::Win32::Security::PSID;
use windows::Win32::Security::SECURITY_ATTRIBUTES;
use windows::Win32::Security::SECURITY_MAX_SID_SIZE;
use windows::Win32::Security::TOKEN_QUERY;
use windows::Win32::Security::TOKEN_USER;
use windows::Win32::Security::TokenUser;
use windows::Win32::Security::WELL_KNOWN_SID_TYPE;
use windows::Win32::Security::WinBuiltinAdministratorsSid;
use windows::Win32::Security::WinBuiltinUsersSid;
use windows::Win32::Security::WinLocalSystemSid;
use windows::Win32::Security::WinWorldSid;
use windows::Win32::Storage::FileSystem::FILE_ALL_ACCESS;
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
use windows::Win32::Storage::FileSystem::FILE_GENERIC_EXECUTE;
use windows::Win32::Storage::FileSystem::FILE_GENERIC_READ;
use windows::Win32::System::Threading::GetCurrentProcess;
use windows::Win32::System::Threading::OpenProcessToken;
use windows::core::Owned;
use windows::core::PCWSTR;
use windows::core::PWSTR;

#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
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
    let token = unsafe { Owned::new(token) };

    let mut required_len = 0u32;
    let _ = unsafe { GetTokenInformation(*token, TokenUser, None, 0, &mut required_len) };
    if required_len == 0 {
        eyre::bail!("Failed determining current token information length");
    }

    let mut buffer = vec![0u8; required_len as usize];
    unsafe {
        GetTokenInformation(
            *token,
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
/// Returns an error if the directory ACL cannot be restricted.
pub fn restrict_path_to_owner(path: &Path, owner_sid: &str) -> eyre::Result<()> {
    set_owner_to_administrators(path)?;
    let entries = [
        AclGrant::full_control(Principal::SidString(owner_sid.to_owned())),
        AclGrant::full_control(Principal::WellKnown(WinLocalSystemSid)),
        AclGrant::full_control(Principal::WellKnown(WinBuiltinAdministratorsSid)),
    ];
    apply_restricted_acl_tree(path, &entries)?;
    Ok(())
}

/// # Errors
///
/// Returns an error if the development read ACL cannot be applied.
pub fn allow_development_reads(path: &Path) -> eyre::Result<()> {
    set_owner_to_administrators(path)?;
    let entries = [
        AclGrant::read_execute(Principal::WellKnown(WinBuiltinUsersSid)),
        AclGrant::read_execute(Principal::WellKnown(WinWorldSid)),
    ];
    apply_acl_grants_tree(path, &entries)?;
    Ok(())
}

/// # Errors
///
/// Returns an error if read/traverse grants for the machine config cannot be applied.
pub fn allow_machine_config_reads(machine_root: &Path, config_path: &Path) -> eyre::Result<()> {
    let entries = [AclGrant::read_execute(Principal::WellKnown(
        WinBuiltinUsersSid,
    ))];
    for path in [machine_root, config_path] {
        let current = read_path_security(path)?;
        apply_acl(path, &entries, current.dacl, false).wrap_err_with(|| {
            format!(
                "Failed applying machine config read ACL for {}",
                path.display()
            )
        })?;
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct AclGrant {
    principal: Principal,
    access_mask: u32,
}

impl AclGrant {
    fn full_control(principal: Principal) -> Self {
        Self {
            principal,
            access_mask: FILE_ALL_ACCESS.0,
        }
    }

    fn read_execute(principal: Principal) -> Self {
        Self {
            principal,
            access_mask: FILE_GENERIC_READ.0 | FILE_GENERIC_EXECUTE.0,
        }
    }
}

#[derive(Debug, Clone)]
enum Principal {
    SidString(String),
    WellKnown(WELL_KNOWN_SID_TYPE),
}

struct OwnedSid {
    ptr: PSID,
    storage: OwnedSidStorage,
}

enum OwnedSidStorage {
    LocalFree,
    Buffer(Vec<u8>),
}

impl OwnedSid {
    fn from_string(sid: &str) -> eyre::Result<Self> {
        let wide_sid = sid.easy_pcwstr()?;
        let mut sid_ptr = PSID::default();
        unsafe { ConvertStringSidToSidW(wide_sid.as_ref(), &mut sid_ptr) }?;
        Ok(Self {
            ptr: sid_ptr,
            storage: OwnedSidStorage::LocalFree,
        })
    }

    fn from_well_known(sid_type: WELL_KNOWN_SID_TYPE) -> eyre::Result<Self> {
        let mut buffer = vec![0u8; SECURITY_MAX_SID_SIZE as usize];
        let mut size = SECURITY_MAX_SID_SIZE;
        let sid_ptr = PSID(buffer.as_mut_ptr().cast());
        unsafe { CreateWellKnownSid(sid_type, None, Some(sid_ptr), &mut size) }?;
        buffer.truncate(size as usize);
        Ok(Self {
            ptr: sid_ptr,
            storage: OwnedSidStorage::Buffer(buffer),
        })
    }

    fn from_principal(principal: &Principal) -> eyre::Result<Self> {
        match principal {
            Principal::SidString(sid) => Self::from_string(sid),
            Principal::WellKnown(sid_type) => Self::from_well_known(*sid_type),
        }
    }

    fn as_psid(&self) -> PSID {
        match &self.storage {
            OwnedSidStorage::LocalFree => self.ptr,
            OwnedSidStorage::Buffer(buffer) => PSID(buffer.as_ptr().cast_mut().cast()),
        }
    }
}

impl Drop for OwnedSid {
    fn drop(&mut self) {
        if matches!(self.storage, OwnedSidStorage::LocalFree) && !self.ptr.is_invalid() {
            let _ = unsafe { LocalFree(Some(HLOCAL(self.ptr.0.cast()))) };
        }
    }
}

struct LocalFreeGuard<T>(*mut T);

impl<T> Drop for LocalFreeGuard<T> {
    fn drop(&mut self) {
        if !self.0.is_null() {
            let _ = unsafe { LocalFree(Some(HLOCAL(self.0.cast()))) };
        }
    }
}

struct SecurityDescriptorGuard(PSECURITY_DESCRIPTOR);

impl Drop for SecurityDescriptorGuard {
    fn drop(&mut self) {
        if !self.0.0.is_null() {
            let _ = unsafe { LocalFree(Some(HLOCAL(self.0.0.cast()))) };
        }
    }
}

fn set_owner_to_administrators(path: &Path) -> eyre::Result<()> {
    let administrators = OwnedSid::from_well_known(WinBuiltinAdministratorsSid)?;
    set_named_security_info(
        path,
        OWNER_SECURITY_INFORMATION,
        Some(administrators.as_psid()),
        None,
    )
}

fn apply_restricted_acl_tree(root: &Path, grants: &[AclGrant]) -> eyre::Result<()> {
    for path in descendant_paths_including_root(root)? {
        apply_acl(&path, grants, None, true)
            .wrap_err_with(|| format!("Failed restricting ACL for {}", path.display()))?;
    }
    Ok(())
}

fn apply_acl_grants_tree(root: &Path, grants: &[AclGrant]) -> eyre::Result<()> {
    for path in descendant_paths_including_root(root)? {
        let current = read_path_security(&path)?;
        apply_acl(&path, grants, current.dacl, false).wrap_err_with(|| {
            format!(
                "Failed applying development read ACL for {}",
                path.display()
            )
        })?;
    }
    Ok(())
}

fn descendant_paths_including_root(root: &Path) -> eyre::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        let metadata = std::fs::symlink_metadata(&path)?;
        let is_reparse_point = metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0;
        let is_dir = metadata.is_dir();
        paths.push(path.clone());
        if !is_dir || is_reparse_point {
            continue;
        }

        for entry in std::fs::read_dir(&path)? {
            stack.push(entry?.path());
        }
    }
    Ok(paths)
}

fn apply_acl(
    path: &Path,
    grants: &[AclGrant],
    old_dacl: Option<*const ACL>,
    protect_dacl: bool,
) -> eyre::Result<()> {
    let sids = grants
        .iter()
        .map(|grant| OwnedSid::from_principal(&grant.principal))
        .collect::<eyre::Result<Vec<_>>>()?;
    let inheritance = OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE;
    let entries = grants
        .iter()
        .zip(&sids)
        .map(|(grant, sid)| EXPLICIT_ACCESS_W {
            grfAccessPermissions: grant.access_mask,
            grfAccessMode: SET_ACCESS,
            grfInheritance: inheritance,
            Trustee: TRUSTEE_W {
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_UNKNOWN,
                ptstrName: PWSTR(sid.as_psid().0.cast()),
                ..Default::default()
            },
        })
        .collect::<Vec<_>>();

    let mut new_acl = std::ptr::null_mut::<ACL>();
    win32_result(
        unsafe { SetEntriesInAclW(Some(&entries), old_dacl, &mut new_acl) },
        "SetEntriesInAclW",
    )?;
    let _new_acl_guard = LocalFreeGuard(new_acl);
    let mut security_info = DACL_SECURITY_INFORMATION;
    if protect_dacl {
        security_info |= PROTECTED_DACL_SECURITY_INFORMATION;
    }
    set_named_security_info(path, security_info, None, Some(new_acl))
}

fn set_named_security_info(
    path: &Path,
    security_info: windows::Win32::Security::OBJECT_SECURITY_INFORMATION,
    owner: Option<PSID>,
    dacl: Option<*const ACL>,
) -> eyre::Result<()> {
    let wide_path = path.easy_pcwstr()?;
    win32_result(
        unsafe {
            SetNamedSecurityInfoW(
                wide_path.as_ref(),
                SE_FILE_OBJECT,
                security_info,
                owner,
                None,
                dacl,
                None,
            )
        },
        "SetNamedSecurityInfoW",
    )
}

/// # Errors
///
/// Returns an error if the ACL cannot be queried.
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

    let security = read_path_security(path)?;
    let owner_sid = OwnedSid::from_string(owner_sid)?;
    let system_sid = OwnedSid::from_well_known(WinLocalSystemSid)?;
    let administrators_sid = OwnedSid::from_well_known(WinBuiltinAdministratorsSid)?;
    let users_sid = OwnedSid::from_well_known(WinBuiltinUsersSid)?;
    let everyone_sid = OwnedSid::from_well_known(WinWorldSid)?;
    let aces = security.explicit_allowed_aces()?;
    Ok(PathProtectionStatus {
        path_exists: true,
        owner_sid_grant_present: aces.iter().any(|ace| ace.sid_equals(owner_sid.as_psid())),
        system_grant_present: aces.iter().any(|ace| ace.sid_equals(system_sid.as_psid())),
        administrators_grant_present: aces
            .iter()
            .any(|ace| ace.sid_equals(administrators_sid.as_psid())),
        broad_read_grant_present: aces.iter().any(|ace| {
            ace.sid_equals(users_sid.as_psid()) || ace.sid_equals(everyone_sid.as_psid())
        }),
        inheritance_disabled: aces.iter().all(|ace| !ace.inherited),
        raw_acl: aces
            .iter()
            .map(AllowedAceView::describe)
            .collect::<Vec<_>>()
            .join("\n"),
    })
}

struct PathSecurity {
    _descriptor: SecurityDescriptorGuard,
    dacl: Option<*const ACL>,
}

impl PathSecurity {
    fn explicit_allowed_aces(&self) -> eyre::Result<Vec<AllowedAceView>> {
        let Some(dacl) = self.dacl else {
            return Ok(Vec::new());
        };
        let mut acl_info = ACL_SIZE_INFORMATION::default();
        #[expect(
            clippy::cast_possible_truncation,
            reason = "ACL_SIZE_INFORMATION size fits in u32"
        )]
        win32_unit(
            unsafe {
                GetAclInformation(
                    dacl,
                    std::ptr::from_mut(&mut acl_info).cast::<c_void>(),
                    std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
                    windows::Win32::Security::AclSizeInformation,
                )
            },
            "GetAclInformation",
        )?;

        let mut aces = Vec::new();
        for ace_index in 0..acl_info.AceCount {
            let mut ace_ptr = std::ptr::null_mut::<c_void>();
            win32_unit(unsafe { GetAce(dacl, ace_index, &mut ace_ptr) }, "GetAce")?;
            let header = unsafe { &*(ace_ptr.cast::<windows::Win32::Security::ACE_HEADER>()) };
            if header.AceType != 0 {
                continue;
            }
            let ace = unsafe { &*(ace_ptr.cast::<ACCESS_ALLOWED_ACE>()) };
            aces.push(AllowedAceView {
                sid: PSID(std::ptr::from_ref(&ace.SidStart).cast_mut().cast()),
                access_mask: ace.Mask,
                inherited: ACE_FLAGS(u32::from(header.AceFlags)).contains(INHERITED_ACE),
            });
        }
        Ok(aces)
    }
}

struct AllowedAceView {
    sid: PSID,
    access_mask: u32,
    inherited: bool,
}

impl AllowedAceView {
    fn sid_equals(&self, other: PSID) -> bool {
        unsafe { EqualSid(self.sid, other) }.is_ok()
    }

    fn describe(&self) -> String {
        let mut sid_string = PWSTR::null();
        let sid = if unsafe { ConvertSidToStringSidW(self.sid, &mut sid_string) }.is_ok() {
            let sid = unsafe { sid_string.to_string() }.unwrap_or_else(|_| String::from("<sid>"));
            let _ = unsafe { LocalFree(Some(HLOCAL(sid_string.0.cast()))) };
            sid
        } else {
            String::from("<sid>")
        };
        format!(
            "{sid}: allow mask=0x{:08x} inherited={}",
            self.access_mask, self.inherited
        )
    }
}

fn read_path_security(path: &Path) -> eyre::Result<PathSecurity> {
    let wide_path = path.easy_pcwstr()?;
    let mut dacl = std::ptr::null_mut::<ACL>();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    win32_result(
        unsafe {
            GetNamedSecurityInfoW(
                wide_path.as_ref(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(&mut dacl),
                None,
                &mut descriptor,
            )
        },
        "GetNamedSecurityInfoW",
    )?;
    Ok(PathSecurity {
        _descriptor: SecurityDescriptorGuard(descriptor),
        dacl: (!dacl.is_null()).then_some(dacl.cast_const()),
    })
}

fn win32_result(error: WIN32_ERROR, operation: &'static str) -> eyre::Result<()> {
    if error.0 == 0 {
        Ok(())
    } else {
        eyre::bail!("{operation} failed with Win32 error {}", error.0)
    }
}

fn win32_unit(result: windows::core::Result<()>, operation: &'static str) -> eyre::Result<()> {
    result.map_err(|error| eyre::eyre!("{operation} failed: {error}"))
}

pub fn warn_if_path_protection_disabled(path: &Path, status: &PathProtectionStatus) {
    if status.protection_enabled() {
        return;
    }
    tracing::warn!(
        sync_dir = %path.display(),
        broad_read_grant_present = status.broad_read_grant_present,
        "Machine cache protection is disabled; this can be expected while developing teamy-mft locally or primarily using teamy-mft without the daemon. Run `teamy-mft protection enable` before normal daemon-backed use."
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
    let wide = sddl.easy_pcwstr()?;
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            wide.as_ref(),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
    }?;

    #[expect(
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
