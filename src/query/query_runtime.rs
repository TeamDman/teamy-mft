use crate::machine::ipc::CorrelationId;
use crate::query::QueryPlan;
use crate::query::QueryResultRow;
use crate::query::QuerySession;
use crate::windows_utils::ctrl_c::GracefulCancellationGuard;
use eyre::WrapErr;
use tracing::info_span;
use std::ops::ControlFlow;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::thread::JoinHandle;
use tracing::debug;

use super::ctrl_c_forwarder::CtrlCForwarder;

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

enum PreparedQueryVisitor {
    Local(LocalQueryVisitor),
    Daemon(DaemonQueryVisitor),
}

struct LocalQueryVisitor {
    _ctrl_c_guard: GracefulCancellationGuard,
    cancel: Arc<AtomicBool>,
    cancel_signal: CtrlCForwarder<()>,
    query_session: QuerySession,
    query_plan: QueryPlan,
}

struct DaemonQueryVisitor {
    rows_rx: vox::Rx<QueryResultRow>,
    cleanup: DaemonQueryCleanup,
}

#[derive(Debug)]
struct DaemonQueryCleanup {
    _ctrl_c_guard: GracefulCancellationGuard,
    response_join: JoinHandle<eyre::Result<CorrelationId>>,
    log_drain: JoinHandle<()>,
    cancel_signal: CtrlCForwarder<eyre::Result<()>>,
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
        mut visit: impl FnMut(QueryResultRow) -> eyre::Result<ControlFlow<(), ()>>,
    ) -> eyre::Result<()> {
        self.visit_rows_dyn(query_plan, &mut visit)
    }

    pub(crate) fn visit_rows_dyn(
        self,
        query_plan: QueryPlan,
        visit: &mut QueryRowVisitor<'_>,
    ) -> eyre::Result<()> {
        let _guard = info_span!("visit_rows_dyn").entered();
        PreparedQueryVisitor::prepare(self, query_plan)?.visit_rows(visit)
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

impl PreparedQueryVisitor {
    fn prepare(runtime: QueryRuntime, query_plan: QueryPlan) -> eyre::Result<Self> {
        match runtime {
            QueryRuntime::Local => {
                Ok(Self::Local(LocalQueryVisitor::prepare(query_plan)?))
            }
            QueryRuntime::DaemonRpc => Ok(Self::Daemon(DaemonQueryVisitor::prepare(query_plan)?)),
        }
    }

    fn visit_rows(self, visit: &mut QueryRowVisitor<'_>) -> eyre::Result<()> {
        match self {
            Self::Local(local) => local.visit_rows(visit),
            Self::Daemon(daemon) => daemon.visit_rows(visit),
        }
    }
}

impl LocalQueryVisitor {
    fn prepare(query_plan: QueryPlan) -> eyre::Result<Self> {
        let ctrl_c_guard = crate::windows_utils::ctrl_c::use_graceful_cancellation();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_signal = CtrlCForwarder::spawn_flag(Arc::clone(&cancel));
        Ok(Self {
            _ctrl_c_guard: ctrl_c_guard,
            cancel,
            cancel_signal,
            query_session: QuerySession::local()?,
            query_plan,
        })
    }

    fn visit_rows(mut self, visit: &mut QueryRowVisitor<'_>) -> eyre::Result<()> {
        let _guard = info_span!("visit_local_rows").entered();
        let result = self.query_session.visit_rows_with_cancel_dyn(
            self.query_plan,
            Some(self.cancel.as_ref()),
            visit,
        );
        self.cancel_signal.finish();
        result
    }
}

impl DaemonQueryVisitor {
    fn prepare(query_plan: QueryPlan) -> eyre::Result<Self> {
        let ctrl_c_guard = crate::windows_utils::ctrl_c::use_graceful_cancellation();
        let config = crate::machine::ipc::load_machine_daemon_client_config()?;
        crate::machine::ipc::ensure_daemon_ready(&config)?;
        let (rows_tx, rows_rx) = vox::channel::<QueryResultRow>();
        let (logs_tx, logs_rx) = vox::channel::<crate::machine::daemon_log::DaemonLogWireEvent>();
        let (cancel_tx, cancel_rx) = vox::channel::<u8>();
        let response_join = std::thread::spawn(move || {
            crate::machine::ipc::query_stream(&config, query_plan, rows_tx, logs_tx, cancel_rx)
                .wrap_err(
                    "Daemon query failed, re-run without `--daemon` to query the published disk cache",
                )
        });
        Ok(Self {
            rows_rx,
            cleanup: DaemonQueryCleanup {
                _ctrl_c_guard: ctrl_c_guard,
                response_join,
                log_drain: crate::machine::daemon_log::spawn_stderr_log_drain(logs_rx),
                cancel_signal: CtrlCForwarder::spawn_sender(cancel_tx),
            },
        })
    }

    fn visit_rows(self, visit: &mut QueryRowVisitor<'_>) -> eyre::Result<()> {
        let _guard = info_span!("visit_daemon_rows").entered();
        let visit_result = QueryRuntime::visit_daemon_rows_from_channel(self.rows_rx, visit);
        if matches!(visit_result, Ok(ControlFlow::Break(()))) {
            self.cleanup.cancel_signal.request_cancel()?;
        }
        let cleanup_result = self.cleanup.finish();
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
    fn finish(self) -> eyre::Result<()> {
        self.cancel_signal.finish()?;
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

#[cfg(test)]
mod tests {
    use super::QueryRuntime;

    #[test]
    fn runtime_constructors_select_expected_backend() {
        assert_eq!(
            QueryRuntime::published_index_only(),
            QueryRuntime::Local
        );
        assert_eq!(QueryRuntime::daemon_rpc(), QueryRuntime::DaemonRpc);
    }
}
