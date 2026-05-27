use crate::windows_utils::invocation::to_args::Invocable;
use crate::windows_utils::invocation::to_args::ToArgs;
use std::ffi::OsString;
use std::path::PathBuf;

/// Unit struct representing the current invocation's arguments with console PID added
#[derive(Debug, Clone)]
pub struct SameInvocationSameConsole;

impl ToArgs for SameInvocationSameConsole {
    fn to_args(&self) -> Vec<OsString> {
        let mut args = std::env::args_os().skip(1).collect::<Vec<_>>();

        // Check if console-pid is already present
        let has_console_pid = args.windows(2).any(|window| {
            if let Some(arg) = window[0].to_str() {
                arg == "--console-pid"
            } else {
                false
            }
        });

        // Add console PID if not already present
        if !has_console_pid {
            args.push("--console-pid".into());
            args.push(std::process::id().to_string().into());
        }

        args
    }
}

impl Invocable for SameInvocationSameConsole {
    fn executable(&self) -> PathBuf {
        std::env::current_exe().expect("Failed to get current executable path")
    }

    fn args(&self) -> Vec<OsString> {
        self.to_args()
    }
}
