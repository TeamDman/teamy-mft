use crate::query::QueryResultRow;

#[derive(Debug, Clone)]
pub struct QueryRowSink {
    tx: tokio::sync::mpsc::Sender<eyre::Result<QueryResultRow>>,
}

impl QueryRowSink {
    #[must_use]
    pub fn new(tx: tokio::sync::mpsc::Sender<eyre::Result<QueryResultRow>>) -> Self {
        Self { tx }
    }

    pub async fn send(&self, row: QueryResultRow) -> bool {
        self.tx.send(Ok(row)).await.is_ok()
    }

    pub async fn send_error(&self, error: eyre::Report) -> bool {
        self.tx.send(Err(error)).await.is_ok()
    }

    /// # Errors
    ///
    /// Returns the row when the receiving stream has been dropped.
    pub fn blocking_send(&self, row: QueryResultRow) -> Result<(), QueryResultRow> {
        self.tx
            .blocking_send(Ok(row))
            .map_err(|error| match error.0 {
                Ok(row) => row,
                Err(_) => unreachable!("sent item was a row"),
            })
    }

    #[must_use]
    pub fn blocking_send_error(&self, error: eyre::Report) -> bool {
        self.tx.blocking_send(Err(error)).is_ok()
    }
}
