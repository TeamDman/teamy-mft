use std::ffi::OsString;
use std::path::PathBuf;

/// Executable and raw Windows parameters for process invocation.
pub trait Invocable {
    fn executable(&self) -> PathBuf;
    /// Parameters excluding the executable, passed to Windows without further quoting.
    fn raw_parameters(&self) -> OsString;
}
