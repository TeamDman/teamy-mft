use crate::daemon::CorrelationId;
use crate::daemon::IndexedPathRowDto;

#[derive(Debug, Clone, PartialEq, Eq, vox::facet::Facet)]
pub struct QueryResponse {
    pub correlation_id: CorrelationId,
    pub rows: Vec<IndexedPathRowDto>,
}
