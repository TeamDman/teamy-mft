#[derive(Debug, Clone, PartialEq, Eq, vox::facet::Facet)]
pub struct UsnJournalRequest {
    pub drive_letter: char,
}
