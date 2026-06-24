use crate::cancellation::CancellationToken;
use crate::machine::ipc::CorrelationId;
use crate::query::QueryPlan;
use crate::query::QueryResultRow;
use crate::query::QuerySession;
use eyre::WrapErr;
use std::fmt;
use std::ops::ControlFlow;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;
use tracing::debug;
use tracing::info_span;

pub type QueryRowVisitor<'a> = dyn FnMut(QueryResultRow) -> eyre::Result<ControlFlow<(), ()>> + 'a;

/// A lightweight backend selector for one-shot queries.
///
/// Use `QueryRuntime` when the caller wants a single query against either the
/// daemon RPC backend or the in-process local backend without holding
/// onto a persistent session. For repeated in-process queries, prefer
/// `QuerySession`.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum QueryRuntime {
    Local,
    DaemonRpc,
}

struct LocalQueryVisitor {
    cancel: CancellationToken,
    query_session: QuerySession,
    query_plan: QueryPlan,
}

struct DaemonQueryVisitor {
    rows_rx: vox::Rx<QueryResultRow>,
    cleanup: DaemonQueryCleanup,
}

struct DaemonQueryCleanup {
    response_join: JoinHandle<eyre::Result<CorrelationId>>,
    log_drain: JoinHandle<()>,
    cancel: CancellationToken,
    cancel_tx: Arc<Mutex<Option<vox::Tx<u8>>>>,
    cancel_watch_stop: mpsc::Sender<()>,
    cancel_watch_join: JoinHandle<eyre::Result<()>>,
}

impl fmt::Debug for DaemonQueryCleanup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DaemonQueryCleanup").finish_non_exhaustive()
    }
}

impl QueryRuntime {
    #[must_use]
    pub const fn published_index_only() -> Self {
        Self::Local
    }

    #[must_use]
    pub const fn daemon_rpc() -> Self {
        Self::DaemonRpc
    }

    /// # Errors
    ///
    /// Returns an error if the configured backend fails while visiting rows.
    pub fn visit_rows(
        self,
        query_plan: QueryPlan,
        cancellation_token: &CancellationToken,
        mut visit: impl FnMut(QueryResultRow) -> eyre::Result<ControlFlow<(), ()>>,
    ) -> eyre::Result<()> {
        self.visit_rows_dyn(query_plan, cancellation_token, &mut visit)
    }

    pub(crate) fn visit_rows_dyn(
        self,
        query_plan: QueryPlan,
        cancellation_token: &CancellationToken,
        visit: &mut QueryRowVisitor<'_>,
    ) -> eyre::Result<()> {
        let _guard = info_span!("visit_rows_dyn").entered();
        match self {
            Self::Local => LocalQueryVisitor::prepare(query_plan, cancellation_token)?.visit_rows(visit),
            Self::DaemonRpc => {
                DaemonQueryVisitor::prepare(query_plan, cancellation_token)?.visit_rows(visit)
            }
        }
    }

    fn visit_daemon_rows_from_channel(
        mut rows_rx: vox::Rx<QueryResultRow>,
        visit: &mut QueryRowVisitor<'_>,
    ) -> eyre::Result<ControlFlow<(), ()>> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async {
            while let Some(row) = match rows_rx.recv().await {
                Ok(Some(row)) => Some(row.get().clone()),
                Ok(None) => None,
                Err(error) => eyre::bail!("Failed receiving streamed query row: {error}"),
            } {
                if visit(row)? == ControlFlow::Break(()) {
                    return eyre::Ok(ControlFlow::Break(()));
                }
            }
            eyre::Ok(ControlFlow::Continue(()))
        })
    }
}

impl LocalQueryVisitor {
    fn prepare(
        query_plan: QueryPlan,
        cancellation_token: &CancellationToken,
    ) -> eyre::Result<Self> {
        Ok(Self {
            cancel: cancellation_token.clone(),
            query_session: QuerySession::local()?,
            query_plan,
        })
    }

    fn visit_rows(mut self, visit: &mut QueryRowVisitor<'_>) -> eyre::Result<()> {
        let _guard = info_span!("visit_local_rows").entered();
        self.query_session
            .visit_rows_dyn(self.query_plan, &self.cancel, visit)
    }
}

impl DaemonQueryVisitor {
    fn prepare(
        query_plan: QueryPlan,
        cancellation_token: &CancellationToken,
    ) -> eyre::Result<Self> {
        let request_cancel = cancellation_token.child_token();
        let config = crate::machine::ipc::load_machine_daemon_client_config()?;
        crate::machine::ipc::ensure_daemon_ready(&config)?;
        let (rows_tx, rows_rx) = vox::channel::<QueryResultRow>();
        let (logs_tx, logs_rx) = vox::channel::<crate::machine::daemon_log::DaemonLogWireEvent>();
        let (cancel_tx, cancel_rx) = vox::channel::<u8>();
        let cancel_tx = Arc::new(Mutex::new(Some(cancel_tx)));
        let (cancel_watch_stop, cancel_watch_stop_rx) = mpsc::channel::<()>();
        let cancel_watch_join = std::thread::spawn({
            let request_cancel = request_cancel.clone();
            let cancel_tx = Arc::clone(&cancel_tx);
            move || {
                loop {
                    if request_cancel.is_cancelled() {
                        return send_daemon_cancel(&cancel_tx);
                    }
                    match cancel_watch_stop_rx.recv_timeout(Duration::from_millis(50)) {
                        Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                }
            }
        });
        let response_join = std::thread::spawn(move || {
            crate::machine::ipc::query_stream(&config, query_plan, rows_tx, logs_tx, cancel_rx)
                .wrap_err(
                    "Daemon query failed, re-run without `--daemon` to query the published disk cache",
                )
        });
        Ok(Self {
            rows_rx,
            cleanup: DaemonQueryCleanup {
                response_join,
                log_drain: crate::machine::daemon_log::spawn_stderr_log_drain(logs_rx),
                cancel: request_cancel,
                cancel_tx,
                cancel_watch_stop,
                cancel_watch_join,
            },
        })
    }

    fn visit_rows(self, visit: &mut QueryRowVisitor<'_>) -> eyre::Result<()> {
        let _guard = info_span!("visit_daemon_rows").entered();
        let visit_result = QueryRuntime::visit_daemon_rows_from_channel(self.rows_rx, visit);
        let cleanup = self.cleanup;
        if matches!(visit_result, Ok(ControlFlow::Break(()))) {
            cleanup.request_cancel("Daemon query stream cancelled by visitor")?;
        }
        let cleanup_result = cleanup.finish();
        match (visit_result, cleanup_result) {
            (Ok(_), Ok(())) => Ok(()),
            (Err(error), Ok(())) => Err(error.wrap_err("visitor failed")),
            (Ok(_), Err(error)) => Err(error.wrap_err("cleanup failed")),
            (Err(visit_error), Err(cleanup_error)) => Err(visit_error
                .wrap_err("visitor failed")
                .wrap_err(format!("cleanup also failed: {cleanup_error}"))),
        }
    }
}

impl DaemonQueryCleanup {
    fn request_cancel(&self, reason: impl Into<String>) -> eyre::Result<()> {
        self.cancel.request_cancel(reason);
        send_daemon_cancel(&self.cancel_tx)
    }

    fn finish(self) -> eyre::Result<()> {
        let _ = self.cancel_watch_stop.send(());
        self.cancel_watch_join.join().map_err(|join_error| {
            eyre::eyre!("Daemon cancel watcher thread panicked: {join_error:?}")
        })??;
        let response = self
            .response_join
            .join()
            .map_err(|join_error| eyre::eyre!("Daemon query thread panicked: {join_error:?}"))??;
        let () = self.log_drain.join().map_err(|join_error| {
            eyre::eyre!("Daemon log drain thread panicked: {join_error:?}")
        })?;
        debug!(
            correlation_id = %response,
            "Daemon-only streamed query completed"
        );
        Ok(())
    }
}

fn send_daemon_cancel(cancel_tx: &Arc<Mutex<Option<vox::Tx<u8>>>>) -> eyre::Result<()> {
    let Some(cancel_tx) = cancel_tx
        .lock()
        .map_err(|poison_error| eyre::eyre!("Daemon cancel sender mutex poisoned: {poison_error}"))?
        .take()
    else {
        return Ok(());
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let _ = cancel_tx.send(1).await;
        let _ = cancel_tx.close(Vec::new()).await;
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::QueryRuntime;

    #[test]
    fn runtime_constructors_select_expected_backend() {
        assert_eq!(QueryRuntime::published_index_only(), QueryRuntime::Local);
        assert_eq!(QueryRuntime::daemon_rpc(), QueryRuntime::DaemonRpc);
    }
}
