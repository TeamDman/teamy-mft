use windows::Win32::System::Console::CONSOLE_SCREEN_BUFFER_INFO;
use windows::Win32::System::Console::GetConsoleScreenBufferInfo;

/// Returns the current console window size in `(columns, rows)`.
///
/// The values mirror the `crossterm` Windows behavior by deriving the visible
/// size from `srWindow` and converting the inclusive coordinates to counts.
#[must_use]
pub fn terminal_size() -> Option<(usize, usize)> {
    let handle = super::get_console_output_handle().ok()?;
    let mut info = CONSOLE_SCREEN_BUFFER_INFO::default();
    unsafe { GetConsoleScreenBufferInfo(handle, &mut info) }.ok()?;
    terminal_size_from_screen_buffer_info(&info)
}

fn terminal_size_from_screen_buffer_info(
    info: &CONSOLE_SCREEN_BUFFER_INFO,
) -> Option<(usize, usize)> {
    let columns = i32::from(info.srWindow.Right) - i32::from(info.srWindow.Left) + 1;
    let rows = i32::from(info.srWindow.Bottom) - i32::from(info.srWindow.Top) + 1;
    if columns <= 0 || rows <= 0 {
        return None;
    }
    Some((usize::try_from(columns).ok()?, usize::try_from(rows).ok()?))
}

#[cfg(test)]
mod tests {
    use super::terminal_size_from_screen_buffer_info;
    use windows::Win32::System::Console::CONSOLE_SCREEN_BUFFER_INFO;
    use windows::Win32::System::Console::SMALL_RECT;

    #[test]
    fn converts_inclusive_console_window_coordinates_to_terminal_counts() {
        let info = CONSOLE_SCREEN_BUFFER_INFO {
            srWindow: SMALL_RECT {
                Left: 10,
                Top: 4,
                Right: 89,
                Bottom: 27,
            },
            ..Default::default()
        };

        assert_eq!(terminal_size_from_screen_buffer_info(&info), Some((80, 24)));
    }

    #[test]
    fn rejects_non_positive_terminal_dimensions() {
        let info = CONSOLE_SCREEN_BUFFER_INFO {
            srWindow: SMALL_RECT {
                Left: 5,
                Top: 3,
                Right: 4,
                Bottom: 10,
            },
            ..Default::default()
        };

        assert_eq!(terminal_size_from_screen_buffer_info(&info), None);
    }
}
