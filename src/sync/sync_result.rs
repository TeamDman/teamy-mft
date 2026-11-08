use crate::domain::DriveLetter;

#[derive(Debug, Clone)]
pub struct SyncResult {
    pub synced_drives: Vec<DriveLetter>,
    pub live_drives: Vec<DriveLetter>,
    pub skipped_drives: Vec<(DriveLetter, String)>,
}
