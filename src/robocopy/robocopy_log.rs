use crate::robocopy::robocopy_header::RobocopyHeader;
use crate::robocopy::robocopy_log_entry::RobocopyLogEntry;

#[derive(Debug)]
pub struct RobocopyLog {
    pub header: RobocopyHeader,
    pub parts: Vec<RobocopyLogEntry>,
}
