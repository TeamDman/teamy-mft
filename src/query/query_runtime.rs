use crate::query::QueryPlan;
use crate::query::QueryResultRow;
use crate::query::QueryRowStream;
use crate::query::QuerySession;
use eyre::WrapErr;
use std::ops::ControlFlow;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tracing::debug;

use super::ctrl_c_forwarder::CtrlCForwarder;

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
pub struct PreparedQueryStream {
    stream: QueryRowStream,
    cleanup: QueryStreamCleanup,
}

#[derive(Debug)]
enum QueryStreamCleanup {
    Local(LocalQueryCleanup),
    Daemon(DaemonQueryCleanup),
}

#[derive(Debug)]
struct LocalQueryCleanup {
    _ctrl_c_guard: crate::windows_utils::ctrl_c::GracefulCancellationGuard,
    query_join: std::thread::JoinHandle<eyre::Result<()>>,
    cancel_signal: CtrlCForwarder<()>,
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
    /// Returns an error if preparing the configured backend fails or if
    /// collecting rows from the resulting stream fails.
    pub fn collect_rows(self, query_plan: QueryPlan) -> eyre::Result<Vec<QueryResultRow>> {
        let limit = query_plan.limit;
        self.prepare_stream(query_plan)?.collect_rows(limit)
    }

    /// # Errors
    ///
    /// Returns an error if the configured backend cannot be prepared.
    pub fn prepare_stream(self, query_plan: QueryPlan) -> eyre::Result<PreparedQueryStream> {
        match self {
            Self::PublishedIndexOnly => Self::prepare_session_query_stream(query_plan),
            Self::DaemonRpc => Self::prepare_daemon_query_stream(query_plan),
        }
    }

    fn prepare_daemon_query_stream(query_plan: QueryPlan) -> eyre::Result<PreparedQueryStream> {
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
        Ok(PreparedQueryStream {
            stream: QueryRowStream::Vox(rows_rx),
            cleanup: QueryStreamCleanup::Daemon(DaemonQueryCleanup {
                _ctrl_c_guard: ctrl_c_guard,
                response_join,
                log_drain: crate::machine::daemon_log::spawn_stderr_log_drain(logs_rx),
                cancel_signal: CtrlCForwarder::spawn_sender(cancel_tx),
            }),
        })
    }

    fn prepare_session_query_stream(query_plan: QueryPlan) -> eyre::Result<PreparedQueryStream> {
        let ctrl_c_guard = crate::windows_utils::ctrl_c::use_graceful_cancellation();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_signal = CtrlCForwarder::spawn_flag(Arc::clone(&cancel));
        let spawned = QuerySession::published_index_only()?.spawn_stream(query_plan, cancel)?;
        Ok(PreparedQueryStream {
            stream: spawned.stream,
            cleanup: QueryStreamCleanup::Local(LocalQueryCleanup {
                _ctrl_c_guard: ctrl_c_guard,
                query_join: spawned.query_join,
                cancel_signal,
            }),
        })
    }
}

impl PreparedQueryStream {
    /// # Errors
    ///
    /// Returns an error if collecting rows from the underlying stream fails or
    /// if backend-specific cleanup fails after collection completes.
    pub fn collect_rows(
        self,
        limit: crate::query::QueryLimit,
    ) -> eyre::Result<Vec<QueryResultRow>> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let rows = runtime.block_on(self.stream.collect_filtered_limit(limit))?;
        self.cleanup.finish()?;
        Ok(rows)
    }

    /// # Errors
    ///
    /// Returns an error if visiting rows from the underlying stream fails or if
    /// backend-specific cleanup fails after visiting completes.
    pub fn visit_rows(
        self,
        mut visit: impl FnMut(QueryResultRow) -> eyre::Result<ControlFlow<(), ()>>,
    ) -> eyre::Result<()> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let PreparedQueryStream {
            mut stream,
            cleanup,
        } = self;
        runtime.block_on(async {
            while let Some(row) = stream.next().await? {
                if visit(row)? == ControlFlow::Break(()) {
                    break;
                }
            }
            Ok::<(), eyre::Report>(())
        })?;
        drop(stream);
        cleanup.finish()?;
        Ok(())
    }
}
impl QueryStreamCleanup {
    fn finish(self) -> eyre::Result<()> {
        match self {
            Self::Local(cleanup) => cleanup.finish(),
            Self::Daemon(cleanup) => cleanup.finish(),
        }
    }
}

impl LocalQueryCleanup {
    fn finish(self) -> eyre::Result<()> {
        self.cancel_signal.finish();
        self.query_join
            .join()
            .map_err(|join_error| eyre::eyre!("Local query thread panicked: {join_error:?}"))??;
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





