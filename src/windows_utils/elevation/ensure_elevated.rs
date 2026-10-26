use crate::windows_utils::elevation::is_elevated;
use crate::windows_utils::elevation::relaunch_as_admin;
use eyre::Context;
use tracing::info;
use tracing::warn;

/// Check if we're elevated, and relaunch if not
pub fn ensure_elevated(console_pid: Option<u32>) -> eyre::Result<()> {
    if is_elevated() {
        return Ok(());
    }
    warn!("Program needs to be run with elevated privileges.");
    info!("Relaunching as administrator...");
    match relaunch_as_admin(console_pid) {
        Ok(child) => {
            info!("Spawned elevated process - waiting for it to finish…");
            let exit_code = child.wait()?;
            info!("Elevated process exited with code {exit_code}");
            std::process::exit(exit_code as i32);
        }
        Err(e) => Err(e).wrap_err("Failed to relaunch as administrator"),
    }
}
