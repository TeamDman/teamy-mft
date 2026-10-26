use crate::windows_utils::elevation::ElevationCompleted;
use crate::windows_utils::elevation::is_elevated;
use crate::windows_utils::elevation::relaunch_as_admin;
use eyre::Context;
use tracing::info;
use tracing::warn;

/// Continue when elevated; otherwise relaunch and return the child's completion signal.
///
/// # Errors
///
/// Returns `ElevationCompleted` after the child exits, or an ordinary error if launching
/// or waiting fails. Callers must propagate the report without converting it to a string.
// cli[impl elevation.completion]
pub fn ensure_elevated(console_pid: Option<u32>) -> eyre::Result<()> {
    if is_elevated() {
        return Ok(());
    }
    warn!("Program needs to be run with elevated privileges.");
    info!("Relaunching as administrator...");
    let child = relaunch_as_admin(console_pid).wrap_err("Failed to relaunch as administrator")?;
    info!("Spawned elevated process - waiting for it to finish…");
    let exit_code = child.wait()?;
    info!("Elevated process exited with code {exit_code}");
    Err(ElevationCompleted { exit_code }.into())
}
