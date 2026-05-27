use crate::query::QueryLimit;
use crate::query::QueryResultRow;

pub enum QueryRowStream {
    Local(tokio::sync::mpsc::Receiver<eyre::Result<QueryResultRow>>),
    Vox(vox::Rx<QueryResultRow>),
}

impl std::fmt::Debug for QueryRowStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local(_) => f.write_str("QueryRowStream::Local(..)"),
            Self::Vox(_) => f.write_str("QueryRowStream::Vox(..)"),
        }
    }
}

impl QueryRowStream {
    /// # Errors
    ///
    /// Returns an error if the local producer failed or the daemon row channel failed.
    pub async fn next(&mut self) -> eyre::Result<Option<QueryResultRow>> {
        match self {
            Self::Local(rx) => rx.recv().await.transpose(),
            Self::Vox(rx) => match rx.recv().await {
                Ok(Some(row)) => Ok(Some(row.get().clone())),
                Ok(None) => Ok(None),
                Err(error) => eyre::bail!("Failed receiving streamed query row: {error}"),
            },
        }
    }

    /// # Errors
    ///
    /// Returns an error if receiving from the underlying stream fails.
    pub async fn collect_filtered_limit(
        mut self,
        limit: QueryLimit,
    ) -> eyre::Result<Vec<QueryResultRow>> {
        let _span = tracing::info_span!("query_collect_results").entered();
        let mut rows = Vec::new();
        if let Some(limit) = **limit {
            let limit = limit.into();
            while let Some(row) = self.next().await? {
                rows.push(row);
                if rows.len() >= limit {
                    break;
                }
            }
        } else {
            while let Some(row) = self.next().await? {
                rows.push(row);
            }
        }
        Ok(rows)
    }
}
