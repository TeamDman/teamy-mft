use crate::windows_utils::console::get_console_output_handle;
use eyre::Context;
use windows::Win32::System::Console::CONSOLE_MODE;
use windows::Win32::System::Console::ENABLE_VIRTUAL_TERMINAL_PROCESSING;
use windows::Win32::System::Console::GetConsoleMode;
use windows::Win32::System::Console::SetConsoleMode;

pub fn enable_ansi_support() -> eyre::Result<()> {
    // Get console handle
    let handle = get_console_output_handle().wrap_err("Failed to get console output handle")?;

    // Get existing mode
    let mut mode = CONSOLE_MODE::default();
    unsafe { GetConsoleMode(handle, &mut mode) }.wrap_err("Failed to get console mode")?;

    // Set new mode to include ANSI support
    unsafe { SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) }
        .wrap_err("Failed to set console mode")?;
    Ok(())
}
