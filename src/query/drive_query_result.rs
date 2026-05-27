use crate::query::QueryResultRow;

#[derive(Debug, Default)]
pub(crate) struct DriveQueryResult {
    pub(crate) loaded_rows: usize,
    pub(crate) matched_rows: Vec<QueryResultRow>,
}
