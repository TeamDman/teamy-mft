use crate::windows_utils::console::console_detach;
use crate::windows_utils::console::enable_ansi_support;
use crate::windows_utils::console::rebind_std_handles_to_console;
use eyre::Context;
use tracing::Level;
use tracing::info;
use tracing::warn;
use windows::Win32::System::Console::*;

/// If called by a new process attaching to an existing process,
/// this should be called before stdout/stderr usage to avoid loss of logs.
///
/// See also: [`ATTACH_PARENT_PROCESS`]
pub fn console_attach(pid: u32) -> eyre::Result<()> {
    let debug_logs_enabled = tracing::event_enabled!(Level::DEBUG);
    if debug_logs_enabled {
        eprintln!("Reusing console with PID: {pid}");
    }

    let _ = console_detach();

    unsafe { AttachConsole(pid) }
        .wrap_err_with(|| format!("Failed to attach to console with PID {pid}."))?;

    rebind_std_handles_to_console()?;

    if let Err(e) = enable_ansi_support() {
        warn!("Failed to enable ANSI support: {:?}", e);
    }

    if pid == ATTACH_PARENT_PROCESS {
        info!("Attached to parent process console");
    } else {
        info!("Attached to console with PID: {pid}", pid = pid.to_string());
    }
    Ok(())
}
