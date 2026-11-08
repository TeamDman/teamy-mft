/// Planned work for a drive, retained unchanged throughout execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncDecision {
    Skip,
    Index,
    MftAndIndex,
}
