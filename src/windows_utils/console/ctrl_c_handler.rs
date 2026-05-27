use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use tracing::debug;
use tracing::error;
use tracing::info;
use windows::Win32::Foundation::*;
use windows::Win32::System::Console::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::BOOL;

static OUR_HWND: AtomicUsize = AtomicUsize::new(0);

pub fn set_our_hwnd(hwnd: HWND) {
    debug!("Writing global HWND: {:?}", hwnd);
    OUR_HWND.store(hwnd.0 as usize, Ordering::SeqCst);
}

pub fn get_our_hwnd() -> Option<HWND> {
    let hwnd_val = OUR_HWND.load(Ordering::SeqCst);
    debug!("Reading global HWND: {:?}", hwnd_val);
    if hwnd_val == 0 {
        None
    } else {
        Some(HWND(hwnd_val as *mut _))
    }
}

/// # Safety
///
/// This function is an extern "system" callback.
pub unsafe extern "system" fn ctrl_c_handler(ctrl_type: u32) -> BOOL {
    match ctrl_type {
        CTRL_C_EVENT | CTRL_BREAK_EVENT | CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT
        | CTRL_SHUTDOWN_EVENT => {
            info!("Received shutdown signal, cleaning up...");
            match get_our_hwnd() {
                Some(hwnd) => {
                    // SendMessageW will synchronously pump the message and wait for it to finish
                    let _result = unsafe { SendMessageW(hwnd, WM_CLOSE, None, None) };
                    TRUE
                }
                None => {
                    error!("No window handle available for cleanup");
                    FALSE
                }
            }
        }
        _ => FALSE,
    }
}

pub fn attach_ctrl_c_handler() -> windows::core::Result<()> {
    debug!("Attaching console ctrl+c handler");
    unsafe {
        SetConsoleCtrlHandler(Some(ctrl_c_handler), true)?;
    }
    Ok(())
}
