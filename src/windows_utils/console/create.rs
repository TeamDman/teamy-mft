use crate::windows_utils::console::attach_ctrl_c_handler;
use crate::windows_utils::console::check_inheriting;
use crate::windows_utils::console::enable_ansi_support;
use crate::windows_utils::console::rebind_std_handles_to_console;
use eyre::Context;
use tracing::error;
use tracing::info;
use windows::Win32::System::Console::AllocConsole;

pub fn console_create() -> eyre::Result<()> {
    // Create new console
    unsafe { AllocConsole() }.wrap_err("Failed to allocate console")?;

    // Important: When launched via tools like `cargo run`, the process may have
    // invalid/redirected std handles. After AllocConsole, rebind them to the
    // newly created console so println!/eprintln! and tracing output go there.
    rebind_std_handles_to_console().wrap_err("Failed to bind std handles to console")?;

    _ = check_inheriting::is_inheriting_console(); // for logging

    // Attach ctrl+c handler (continue on error)
    if let Err(e) = attach_ctrl_c_handler().wrap_err("Failed to set console control handler") {
        error!("{:?}", e);
    }

    // Enable ANSI support (continue on error)
    if let Err(e) = enable_ansi_support().wrap_err("Failed to enable ANSI support") {
        error!("{:?}", e);
    }

    // Tell the user whats up
    info!("Console allocated, new logs will be visible here.");
    info!("Closing this window will exit the program.");
    Ok(())
}
