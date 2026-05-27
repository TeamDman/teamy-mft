use crate::windows_utils::console::set_our_hwnd;
use crate::windows_utils::module::get_current_module;
use tracing::debug;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

/// Creates a window for message pumping for tray icon interactions.
/// <https://learn.microsoft.com/en-us/windows/win32/winmsg/about-messages-and-message-queues>
pub fn create_window_for_tray(window_proc: WNDPROC) -> eyre::Result<HWND> {
    debug!("Creating hidden window for tray icon");
    let instance = get_current_module()?;
    let class_name = w!("TrayIconWindow");

    let window_class = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: window_proc,
        hInstance: instance.into(),
        lpszClassName: class_name,
        ..Default::default()
    };

    debug!(class_name = ?class_name, "Registering window class");
    let atom = unsafe { RegisterClassExW(&window_class) };
    std::debug_assert_ne!(atom, 0);

    let window_title = w!("Tray Icon");
    debug!(title = ?window_title, "Creating window");
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class_name,
            window_title,
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            None,
            None,
            Some(instance.into()),
            None,
        )
    }?;

    set_our_hwnd(hwnd);

    Ok(hwnd)
}
