#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IndexedPathRow {
    pub path: String,
    pub has_deleted_entries: bool,
    pub is_ignored: bool,
}
