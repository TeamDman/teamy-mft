/// Signals that an elevated child handled the command and the parent should finish.
/// This travels through `eyre::Result` so callers return normally and drop their resources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElevationCompleted {
    pub exit_code: u32,
}

impl std::fmt::Display for ElevationCompleted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Elevated child completed with exit code {}",
            self.exit_code
        )
    }
}

impl std::error::Error for ElevationCompleted {}
