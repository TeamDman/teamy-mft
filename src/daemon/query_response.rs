use crate::daemon::CorrelationId;
use crate::query::QueryResultRow;

#[derive(Debug, Clone, PartialEq, Eq, vox::facet::Facet)]
pub struct QueryResponse {
    pub correlation_id: CorrelationId,
    pub rows: Vec<QueryResultRow>,
}
