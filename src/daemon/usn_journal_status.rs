#[derive(Debug, Clone, PartialEq, Eq, vox::facet::Facet)]
pub struct UsnJournalStatus {
    pub drive_letter: char,
    pub active: bool,
    pub journal_id: Option<u64>,
    pub first_usn: Option<u64>,
    pub next_usn: Option<u64>,
    pub lowest_valid_usn: Option<u64>,
    pub max_usn: Option<u64>,
    pub inactive_reason: Option<String>,
}
