use crate::windows_utils::elevation::ElevatedChildProcess;
use crate::windows_utils::invocation::Invocable;
use crate::windows_utils::string::EasyPCWSTR;
use eyre::Context;
use std::ffi::OsString;
use windows::Win32::UI::Shell::SEE_MASK_NOCLOSEPROCESS;
use windows::Win32::UI::Shell::SHELLEXECUTEINFOW;
use windows::Win32::UI::Shell::ShellExecuteExW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// Runs an invocable with administrative privileges using ShellExecuteExW.
pub fn run_as_admin(invocable: &impl Invocable) -> eyre::Result<ElevatedChildProcess> {
    // Build a single space-separated string of arguments
    let params: OsString = invocable
        .args()
        .into_iter()
        .fold(OsString::new(), |mut acc, arg| {
            acc.push(arg);
            acc.push(" ");
            acc
        });

    // ---------------- ShellExecuteExW ----------------
    let verb = "runas".easy_pcwstr()?;
    let file = invocable.executable().easy_pcwstr()?;
    let params = params.easy_pcwstr()?;

    let mut sei = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: unsafe { verb.as_ptr() },
        lpFile: unsafe { file.as_ptr() },
        lpParameters: unsafe { params.as_ptr() },
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut sei) }.wrap_err("Failed to run as administrator")?;
    Ok(ElevatedChildProcess {
        h_process: sei.hProcess,
    })
}
