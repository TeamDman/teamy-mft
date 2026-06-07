use windows::Win32::Security::CheckTokenMembership;
use windows::Win32::Security::CreateWellKnownSid;
use windows::Win32::Security::PSID;
use windows::Win32::Security::SECURITY_MAX_SID_SIZE;
use windows::Win32::Security::WinBuiltinAdministratorsSid;
use windows::core::BOOL;

/// # Errors
///
/// Returns an error if Windows cannot determine whether the current token is in
/// `BUILTIN\Administrators`.
pub fn is_in_builtin_administrators() -> eyre::Result<bool> {
    let mut sid_buffer = vec![0u8; SECURITY_MAX_SID_SIZE as usize];
    let mut sid_len = SECURITY_MAX_SID_SIZE;
    let administrators_sid = PSID(sid_buffer.as_mut_ptr().cast());
    unsafe {
        CreateWellKnownSid(
            WinBuiltinAdministratorsSid,
            None,
            Some(administrators_sid),
            &mut sid_len,
        )
    }?;

    let mut is_member = BOOL(0);
    unsafe { CheckTokenMembership(None, administrators_sid, &mut is_member) }?;
    Ok(is_member.as_bool())
}
