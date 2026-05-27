use crate::windows_utils::console::attach_ctrl_c_handler;
use crate::windows_utils::console::console_detach;
use crate::windows_utils::console::is_inheriting_console;

pub fn hide_default_console_or_attach_ctrl_handler() -> eyre::Result<()> {
    if is_inheriting_console() {
        // There is an existing console (e.g., VSCode), so attach ctrl+c handler for graceful shutdowns
        attach_ctrl_c_handler()?;
    } else {
        // No existing console (e.g., double-clicked exe), so detach from the default console window
        // No need for ctrl+c handler since there is no console to send ctrl+c to
        _ = console_detach();
    };

    Ok(())
}
