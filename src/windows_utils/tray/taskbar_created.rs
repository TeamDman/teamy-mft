use std::sync::LazyLock;
use windows::Win32::UI::WindowsAndMessaging::RegisterWindowMessageW;
use windows::core::w;

/// Returns the atom/message ID for the "TaskbarCreated" broadcast message.
/// Explorer broadcasts this after the taskbar is (re)created, e.g., after a crash.
/// Apps should re-add their tray icons when they receive this message.
pub static WM_TASKBAR_CREATED: LazyLock<u32> =
    LazyLock::new(|| unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) });
