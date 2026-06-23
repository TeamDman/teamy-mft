use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

static INTERRUPTED: AtomicBool = AtomicBool::new(false);
static LAST_INTERRUPT_UNIX_MS: AtomicU64 = AtomicU64::new(0);
static GRACEFUL_CANCELLATION_ENABLED: AtomicBool = AtomicBool::new(false);

#[cfg(windows)]
const DOUBLE_INTERRUPT_WINDOW_MS: u64 = 1_000;

#[cfg(windows)]
const CTRL_C_EXIT_CODE: i32 = 130;

/// # Errors
///
/// Returns an error if the Windows console control handler cannot be installed.
#[cfg(windows)]
pub fn install_ctrl_c_handler() -> eyre::Result<()> {
    // SAFETY: `handle_console_control` has the required system ABI and is valid
    // for the lifetime of the process.
    unsafe {
        windows::Win32::System::Console::SetConsoleCtrlHandler(Some(handle_console_control), true)?;
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn install_ctrl_c_handler() -> eyre::Result<()> {
    Ok(())
}

#[must_use]
pub fn interrupted() -> bool {
    INTERRUPTED.load(Ordering::Relaxed)
}

pub fn request_interrupt_from_process_handler() {
    INTERRUPTED.store(true, Ordering::Relaxed);
}

#[must_use]
pub fn use_graceful_cancellation() -> GracefulCancellationGuard {
    INTERRUPTED.store(false, Ordering::Relaxed);
    LAST_INTERRUPT_UNIX_MS.store(0, Ordering::Relaxed);
    GRACEFUL_CANCELLATION_ENABLED.store(true, Ordering::Relaxed);
    GracefulCancellationGuard
}

#[derive(Debug)]
pub struct GracefulCancellationGuard;

impl Drop for GracefulCancellationGuard {
    fn drop(&mut self) {
        GRACEFUL_CANCELLATION_ENABLED.store(false, Ordering::Relaxed);
    }
}

#[cfg(windows)]
unsafe extern "system" fn handle_console_control(ctrl_type: u32) -> windows::core::BOOL {
    if ctrl_type != windows::Win32::System::Console::CTRL_C_EVENT {
        return false.into();
    }

    let now = crate::machine::config::current_unix_ms();
    let previous = LAST_INTERRUPT_UNIX_MS.swap(now, Ordering::Relaxed);
    eprintln!("\x1b[31m^C\x1b[0m");

    if !GRACEFUL_CANCELLATION_ENABLED.load(Ordering::Relaxed) {
        return false.into();
    }

    if previous != 0 && now.saturating_sub(previous) <= DOUBLE_INTERRUPT_WINDOW_MS {
        std::process::exit(CTRL_C_EXIT_CODE);
    }

    INTERRUPTED.store(true, Ordering::Relaxed);
    true.into()
}
