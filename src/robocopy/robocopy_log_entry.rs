use chrono::DateTime;
use chrono::Local;
use std::path::PathBuf;
use uom::si::u64::Information;

#[derive(Debug, PartialEq, Eq)]
pub enum RobocopyLogEntry {
    AccessDeniedError {
        when: DateTime<Local>,
        path: PathBuf,
    },
    NewFile {
        size: Information,
        path: PathBuf,
        percentages: Vec<u8>,
    },
}
