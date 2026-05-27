use std::ffi::OsString;
use std::path::PathBuf;

/// Trait for converting CLI structures to command line arguments
pub trait ToArgs {
    fn to_args(&self) -> Vec<OsString> {
        Vec::new()
    }
}

// Blanket implementation for references
impl<T: ToArgs> ToArgs for &T {
    fn to_args(&self) -> Vec<OsString> {
        (*self).to_args()
    }
}

/// Trait for providing executable and arguments for process invocation
pub trait Invocable {
    fn executable(&self) -> PathBuf;
    fn args(&self) -> Vec<OsString>;
}
