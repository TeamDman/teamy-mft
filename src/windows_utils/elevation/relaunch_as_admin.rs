use crate::windows_utils::elevation::ElevatedChildProcess;
use crate::windows_utils::elevation::run_as_admin;
use crate::windows_utils::invocation::SameInvocationSameConsole;

/// Relaunches the current executable with administrative privileges, preserving arguments and console.
/// `console_pid` must be the value already parsed from the current invocation.
pub fn relaunch_as_admin(console_pid: Option<u32>) -> eyre::Result<ElevatedChildProcess> {
    run_as_admin(&SameInvocationSameConsole { console_pid })
}
