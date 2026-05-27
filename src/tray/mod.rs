mod window_proc;

use eyre::Context;
use std::ffi::c_void;
use std::sync::Mutex;
use std::sync::OnceLock;
use crate::windows_utils::console::hide_default_console_or_attach_ctrl_handler;
use crate::windows_utils::event_loop::run_message_loop;
use crate::windows_utils::hicon::application_icon::get_application_icon;
use crate::windows_utils::tray::TRAY_ICON_ID;
use crate::windows_utils::tray::add_tray_icon;
use crate::windows_utils::window::create_window_for_tray;
use tracing::info;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::NIF_ICON;
use windows::Win32::UI::Shell::NIM_MODIFY;
use windows::Win32::UI::Shell::NOTIFYICONDATAW;
use windows::Win32::UI::Shell::Shell_NotifyIconW;
use windows::Win32::UI::WindowsAndMessaging::HICON;
use windows::core::w;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrayContext {
    pub inherited_console_available: bool,
}

static TRAY_WINDOW: OnceLock<isize> = OnceLock::new();
static CURRENT_ICON: OnceLock<Mutex<Option<isize>>> = OnceLock::new();

fn current_icon_slot() -> &'static Mutex<Option<isize>> {
    CURRENT_ICON.get_or_init(|| Mutex::new(None))
}

#[must_use]
pub fn current_tray_icon() -> Option<HICON> {
    current_icon_slot()
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().copied())
        .map(|bits| HICON(bits as *mut c_void))
}

fn record_tray_window(hwnd: HWND) {
    let _ = TRAY_WINDOW.set(hwnd.0 as isize);
}

fn record_tray_icon(icon: HICON) {
    *current_icon_slot()
        .lock()
        .expect("tray icon mutex poisoned") = Some(icon.0 as isize);
}

/// # Errors
///
/// Returns an error if the tray icon cannot be updated.
///
/// # Panics
///
/// Panics if the `NOTIFYICONDATAW` size ever stops fitting in `u32`.
pub fn set_tray_icon(icon: HICON) -> eyre::Result<()> {
    record_tray_icon(icon);
    let hwnd_bits = *TRAY_WINDOW
        .get()
        .ok_or_else(|| eyre::eyre!("Tray window handle not available"))?;
    let hwnd = HWND(hwnd_bits as *mut c_void);

    let notify_icon_data = NOTIFYICONDATAW {
        cbSize: u32::try_from(std::mem::size_of::<NOTIFYICONDATAW>())
            .expect("NOTIFYICONDATAW size fits in u32"),
        hWnd: hwnd,
        uID: TRAY_ICON_ID,
        uFlags: NIF_ICON,
        hIcon: icon,
        ..Default::default()
    };

    // SAFETY: The tray window handle and icon data remain valid for the duration of the shell notification call.
    unsafe { Shell_NotifyIconW(NIM_MODIFY, &raw const notify_icon_data) }
        .ok()
        .wrap_err("Failed to update tray icon")?;

    Ok(())
}

/// # Errors
///
/// Returns an error if tray initialization or its message loop fails.
pub fn run_tray(context: TrayContext) -> eyre::Result<()> {
    hide_default_console_or_attach_ctrl_handler()?;

    window_proc::configure(window_proc::TrayWindowConfig {
        inherited_console_available: context.inherited_console_available,
    })?;

    let window = create_window_for_tray(Some(window_proc::window_proc))?;
    record_tray_window(window);

    let icon = get_application_icon()?;
    record_tray_icon(icon);
    add_tray_icon(window, icon, w!("teamy-mft"))?;

    info!("Tray initialized");
    run_message_loop(None)?;
    Ok(())
}
