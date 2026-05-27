use crate::windows_utils::invocation::to_args::Invocable;
use crate::windows_utils::invocation::to_args::ToArgs;
use std::ffi::OsString;
use std::path::PathBuf;

/// Unit struct representing the current invocation's arguments
#[derive(Debug, Clone)]
pub struct SameInvocation;

impl ToArgs for SameInvocation {
    fn to_args(&self) -> Vec<OsString> {
        std::env::args_os().skip(1).collect()
    }
}

impl Invocable for SameInvocation {
    fn executable(&self) -> PathBuf {
        std::env::current_exe().expect("Failed to get current executable path")
    }

    fn args(&self) -> Vec<OsString> {
        std::env::args_os().skip(1).collect()
    }
}
