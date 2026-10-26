use crate::windows_utils::invocation::Invocable;
use crate::windows_utils::invocation::same_invocation::SameInvocation;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::path::PathBuf;

/// The current invocation with a console PID added when the CLI did not supply one.
#[derive(Debug, Clone)]
pub struct SameInvocationSameConsole {
    pub console_pid: Option<u32>,
}

impl Invocable for SameInvocationSameConsole {
    fn executable(&self) -> PathBuf {
        std::env::current_exe().expect("Failed to get current executable path")
    }

    fn raw_parameters(&self) -> OsString {
        console_parameters(
            &SameInvocation.raw_parameters(),
            self.console_pid,
            std::process::id(),
        )
    }
}

// cli[impl elevation.console-pid]
fn console_parameters(raw_parameters: &OsStr, console_pid: Option<u32>, pid: u32) -> OsString {
    let mut parameters = if console_pid.is_some() {
        OsString::new()
    } else {
        // Prepend so `--` or an unclosed quote in the original text cannot consume it.
        OsString::from(format!("--console-pid {pid} "))
    };
    parameters.push(raw_parameters);
    parameters
}

#[cfg(test)]
mod tests {
    use super::console_parameters;
    use std::ffi::OsStr;
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::ffi::OsStringExt;

    // cli[verify elevation.console-pid]
    #[test]
    fn prepends_console_pid_without_rewriting_the_original_text() {
        for (raw, expected) in [
            ("", "--console-pid 42 "),
            ("sync --no-daemon", "--console-pid 42 sync --no-daemon"),
            (
                r#"query ""  "a\"b" "unfinished"#,
                r#"--console-pid 42 query ""  "a\"b" "unfinished"#,
            ),
            (
                "query -- --console-pid",
                "--console-pid 42 query -- --console-pid",
            ),
            (
                "query -- --console-pid=7",
                "--console-pid 42 query -- --console-pid=7",
            ),
        ] {
            assert_eq!(console_parameters(OsStr::new(raw), None, 42), expected,);
        }
    }

    #[test]
    fn retains_existing_console_pid_in_both_forms() {
        for (raw, args) in [
            ("sync --console-pid 7", vec!["sync", "--console-pid", "7"]),
            ("--console-pid=7 sync", vec!["--console-pid=7", "sync"]),
            (
                r#"sync "--console-pid" "7""#,
                vec!["sync", "--console-pid", "7"],
            ),
            (
                "--console-pid 7 query -- --console-pid=8",
                vec!["--console-pid", "7", "query", "--", "--console-pid=8"],
            ),
        ] {
            let cli: crate::cli::Cli = figue::from_slice(&args).unwrap();
            assert_eq!(cli.global_args.console_pid, Some(7));
            assert_eq!(
                console_parameters(OsStr::new(raw), cli.global_args.console_pid, 42),
                raw,
            );
        }
    }

    #[test]
    fn prefix_preserves_unpaired_surrogates() {
        let raw = OsString::from_wide(&[0xd800, u16::from(b' '), 0xdc00]);
        let parameters = console_parameters(&raw, None, 42);
        let mut expected = "--console-pid 42 ".encode_utf16().collect::<Vec<_>>();
        expected.extend(raw.encode_wide());
        assert_eq!(parameters.encode_wide().collect::<Vec<_>>(), expected);
    }

    #[test]
    fn console_pid_detection_follows_cli_option_values() {
        for (raw, args, expected) in [
            (
                "--log-file --console-pid sync",
                vec!["--log-file", "--console-pid", "sync"],
                "--console-pid 42 --log-file --console-pid sync",
            ),
            (
                "--log-file -- sync --console-pid 7",
                vec!["--log-file", "--", "sync", "--console-pid", "7"],
                "--log-file -- sync --console-pid 7",
            ),
        ] {
            // Both commands are valid: the apparent flag/terminator is a log-file value.
            let cli: crate::cli::Cli = figue::from_slice(&args).unwrap();
            assert_eq!(
                console_parameters(OsStr::new(raw), cli.global_args.console_pid, 42),
                expected,
            );
        }
    }

    #[test]
    fn console_pid_after_terminator_is_positional() {
        for raw in ["--console-pid", "--console-pid=7"] {
            let cli: crate::cli::Cli = figue::from_slice(&["query", "--", raw]).unwrap();
            assert_eq!(cli.global_args.console_pid, None);
            let parameters = format!("query -- {raw}");
            assert_eq!(
                console_parameters(OsStr::new(&parameters), cli.global_args.console_pid, 42),
                OsString::from(format!("--console-pid 42 {parameters}")),
            );
        }
    }
}
