use crate::query::QuerySource;

#[derive(Debug, Clone)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "Query request spec carries independent CLI/RPC filtering switches"
)]
pub struct QueryRequestSpec {
    pub query: Vec<String>,
    pub query_scope: Option<String>,
    pub drive_letters: Vec<char>,
    pub limit: usize,
    pub include_deleted: bool,
    pub only_deleted: bool,
    pub show_ignored: bool,
    pub only_ignored: bool,
    pub source: QuerySource,
    pub allow_fallback: bool,
}

impl From<&teamy_mft_daemon_rpc::QueryRequest> for QueryRequestSpec {
    fn from(value: &teamy_mft_daemon_rpc::QueryRequest) -> Self {
        Self {
            query: value.query.clone(),
            query_scope: value.query_scope.clone(),
            drive_letters: value.drive_letters.clone(),
            limit: value.limit,
            include_deleted: value.include_deleted,
            only_deleted: value.only_deleted,
            show_ignored: value.show_ignored,
            only_ignored: value.only_ignored,
            source: QuerySource::DaemonOnly,
            allow_fallback: false,
        }
    }
}
