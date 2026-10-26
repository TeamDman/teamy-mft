use crate::windows_utils::invocation::Invocable;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use windows::Win32::System::Environment::GetCommandLineW;

/// Unit struct representing the current invocation's arguments
#[derive(Debug, Clone)]
pub struct SameInvocation;

impl Invocable for SameInvocation {
    fn executable(&self) -> PathBuf {
        std::env::current_exe().expect("Failed to get current executable path")
    }

    fn raw_parameters(&self) -> OsString {
        // SAFETY: Windows owns this null-terminated buffer; we only read and copy it.
        unsafe {
            let command_line = GetCommandLineW();
            OsString::from_wide(command_line_parameters(command_line.as_wide()))
        }
    }
}

// cli[impl elevation.raw-arguments]
fn command_line_parameters(command_line: &[u16]) -> &[u16] {
    const QUOTE: u16 = b'"' as u16;
    const SPACE: u16 = b' ' as u16;
    const TAB: u16 = b'\t' as u16;

    // GetCommandLineW includes the executable; ShellExecuteExW takes it separately.
    // Scan argv[0] only: quotes protect spaces in its path, and backslashes are literal.
    // Skip its separating whitespace and return the remaining UTF-16 units unchanged.
    let mut in_quotes = false;
    let end = command_line
        .iter()
        .position(|&unit| match unit {
            QUOTE => {
                in_quotes = !in_quotes;
                false
            }
            SPACE | TAB => !in_quotes,
            _ => false,
        })
        .unwrap_or(command_line.len());
    let tail = &command_line[end..];
    let start = tail
        .iter()
        .position(|&unit| !matches!(unit, SPACE | TAB))
        .unwrap_or(tail.len());
    &tail[start..]
}

#[cfg(test)]
mod tests {
    use super::command_line_parameters;

    // cli[verify elevation.raw-arguments]
    #[test]
    fn skips_only_the_executable_and_its_separator() {
        for (command_line, expected) in [
            ("", ""),
            ("tool.exe", ""),
            ("tool.exe \t ", ""),
            ("tool.exe sync --no-daemon", "sync --no-daemon"),
            (r#""C:\Program Files\tool.exe" sync"#, "sync"),
            (r#""C:\Program Files"\tool.exe sync"#, "sync"),
            (
                "tool.exe\t\t sync  \t--no-daemon  ",
                "sync  \t--no-daemon  ",
            ),
            (r#""tool.exe"sync --no-daemon"#, "--no-daemon"),
            (r#""tool.exe sync"#, ""),
            // Rust treats leading whitespace as an empty argv[0].
            (" tool.exe sync", "tool.exe sync"),
        ] {
            let command_line = command_line.encode_utf16().collect::<Vec<_>>();
            assert_eq!(
                command_line_parameters(&command_line),
                expected.encode_utf16().collect::<Vec<_>>(),
            );
        }
    }

    #[test]
    fn preserves_quotes_backslashes_unicode_and_unclosed_quotes() {
        for tail in [
            r#"sync "C:\Some Folder\""#,
            r#"query "" "a\"b" "ends in slash\\""#,
            "query \"snowman ☃\"\t\"\"  ",
            r#"query "unfinished argument"#,
            "query -- --console-pid",
        ] {
            let command_line = format!(r#""C:\Program Files\tool.exe" {tail}"#)
                .encode_utf16()
                .collect::<Vec<_>>();
            assert_eq!(
                command_line_parameters(&command_line),
                tail.encode_utf16().collect::<Vec<_>>(),
            );
        }
    }

    #[test]
    fn preserves_unpaired_surrogates_and_long_arguments() {
        let mut command_line = "tool.exe ".encode_utf16().collect::<Vec<_>>();
        let mut tail = vec![0xd800];
        tail.extend(std::iter::repeat_n(u16::from(b'x'), 4096));
        tail.push(0xdc00);
        command_line.extend_from_slice(&tail);
        assert_eq!(command_line_parameters(&command_line), tail);
    }
}
