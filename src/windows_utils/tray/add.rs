use core::ffi::c_void;
use eyre::Context;
use std::sync::Mutex;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::PCWSTR;
use windows::core::Param;
use windows::core::ParamValue;

/// <https://learn.microsoft.com/en-us/windows/win32/winmsg/wm-user>
pub const WM_USER_TRAY_CALLBACK: u32 = WM_USER + 1;
pub const TRAY_ICON_ID: u32 = 1;

// Minimal, Send-friendly state to reconstruct the tray icon after Explorer restarts.
#[derive(Clone, Copy)]
struct MinimalTrayState {
    hwnd_bits: isize,
    hicon_bits: isize,
    tip: [u16; 128],
}

static TRAY_STATE: Mutex<Option<MinimalTrayState>> = Mutex::new(None);

/// Adds a tray icon to the system tray with the specified icon and tooltip.
/// Returns the NOTIFYICONDATAW used to create the icon, which can be used for further modifications.
pub fn add_tray_icon(
    hwnd: HWND,
    icon: HICON,
    tooltip: impl Param<PCWSTR>,
) -> eyre::Result<NOTIFYICONDATAW> {
    // Create tray icon
    let mut notify_icon_data = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ICON_ID,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_USER_TRAY_CALLBACK,
        hIcon: icon,
        szTip: [0; 128],
        ..Default::default()
    };

    // Set tooltip
    let tooltip: ParamValue<PCWSTR> = unsafe { tooltip.param() };
    let tooltip = tooltip.abi();
    let tooltip = unsafe { tooltip.as_wide() };
    notify_icon_data.szTip[..tooltip.len()].copy_from_slice(tooltip);

    // Add the icon to the system tray
    unsafe { Shell_NotifyIconW(NIM_ADD, &notify_icon_data).ok() }
        .wrap_err("Failed to add tray icon")?;

    // Save state for potential re-add after TaskbarCreated
    {
        let mut guard = TRAY_STATE.lock().unwrap();
        *guard = Some(MinimalTrayState {
            hwnd_bits: hwnd.0 as isize,
            hicon_bits: icon.0 as isize,
            tip: notify_icon_data.szTip,
        });
    }

    Ok(notify_icon_data)
}

/// Re-add the tray icon using the last known NOTIFYICONDATAW.
/// Call this when the system broadcasts the TaskbarCreated message.
pub fn re_add_tray_icon() -> eyre::Result<()> {
    let saved = {
        let guard = TRAY_STATE.lock().unwrap();
        *guard
    };
    if let Some(state) = saved {
        let nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: HWND(state.hwnd_bits as *mut c_void),
            uID: TRAY_ICON_ID,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: WM_USER_TRAY_CALLBACK,
            hIcon: HICON(state.hicon_bits as *mut c_void),
            szTip: state.tip,
            ..Default::default()
        };
        unsafe { Shell_NotifyIconW(NIM_ADD, &nid).ok() }.wrap_err("Failed to re-add tray icon")?;
        Ok(())
    } else {
        Err(eyre::eyre!("No tray state available to re-add icon"))
    }
}
