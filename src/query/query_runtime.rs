use crate::query::QueryPlan;
use crate::query::QueryResultRow;
use crate::query::QuerySession;
use eyre::WrapErr;
use std::ops::ControlFlow;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tracing::debug;

use super::ctrl_c_forwarder::CtrlCForwarder;

pub(crate) type QueryRowVisitor<'a> =
    dyn FnMut(QueryResultRow) -> eyre::Result<ControlFlow<(), ()>> + 'a;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
/// A lightweight backend selector for one-shot queries.
///
/// Use `QueryRuntime` when the caller wants a single query against either the
/// daemon RPC backend or the in-process published-index backend without holding
/// onto a persistent session. For repeated in-process queries, prefer
/// `QuerySession`.
pub enum QueryRuntime {
    PublishedIndexOnly,
    DaemonRpc,
}

#[derive(Debug)]
struct DaemonQueryCleanup {
    _ctrl_c_guard: crate::windows_utils::ctrl_c::GracefulCancellationGuard,
    response_join: std::thread::JoinHandle<eyre::Result<crate::machine::ipc::CorrelationId>>,
    log_drain: std::thread::JoinHandle<()>,
    cancel_signal: CtrlCForwarder<eyre::Result<()>>,
}

impl QueryRuntime {
    #[must_use]
    pub const fn published_index_only() -> Self {
        Self::PublishedIndexOnly
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
        match self {
            Self::PublishedIndexOnly => Self::visit_in_current_process_query_rows(query_plan, visit),
            Self::DaemonRpc => Self::visit_daemon_query_rows(query_plan, visit),
        }
    }

    fn visit_daemon_query_rows(
        query_plan: QueryPlan,
        visit: &mut QueryRowVisitor<'_>,
    ) -> eyre::Result<()> {
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
        let cleanup = DaemonQueryCleanup {
            _ctrl_c_guard: ctrl_c_guard,
            response_join,
            log_drain: crate::machine::daemon_log::spawn_stderr_log_drain(logs_rx),
            cancel_signal: CtrlCForwarder::spawn_sender(cancel_tx),
        };

        let visit_result = Self::visit_daemon_rows_from_channel(rows_rx, visit);
        let cleanup_result = cleanup.finish();
        visit_result?;
        cleanup_result
    }

    fn visit_in_current_process_query_rows(
        query_plan: QueryPlan,
        visit: &mut QueryRowVisitor<'_>,
    ) -> eyre::Result<()> {
        let mut query_session = QuerySession::in_current_process()?;
        let _ctrl_c_guard = crate::windows_utils::ctrl_c::use_graceful_cancellation();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_signal = CtrlCForwarder::spawn_flag(Arc::clone(&cancel));
        let result = query_session.visit_rows_with_cancel_dyn(query_plan, Some(cancel.as_ref()), visit);
        cancel_signal.finish();
        result
    }

    fn visit_daemon_rows_from_channel(
        mut rows_rx: vox::Rx<QueryResultRow>,
        visit: &mut QueryRowVisitor<'_>,
    ) -> eyre::Result<()> {
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
                    break;
                }
            }
            Ok::<(), eyre::Report>(())
        })?;
        Ok(())
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
            QueryRuntime::PublishedIndexOnly
        );
        assert_eq!(QueryRuntime::daemon_rpc(), QueryRuntime::DaemonRpc);
    }
}





