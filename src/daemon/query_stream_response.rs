use crate::daemon::CorrelationId;

#[derive(Debug, Clone, PartialEq, Eq, vox::facet::Facet)]
pub struct QueryStreamResponse {
    pub correlation_id: CorrelationId,
}
