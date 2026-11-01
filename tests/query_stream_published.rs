//! Regression for the published Phon explicit-proxy schema and RPC path.
#![allow(dead_code)]

pub use teamy_mft::daemon;
pub use teamy_mft::query;
pub use teamy_mft::sync;

#[path = "../src/daemon/machine_daemon_rpc.rs"]
mod actual_rpc;

#[cfg(test)]
mod tests {
    use super::*;
    use daemon::*;
    use facet::Facet;
    use query::QueryLimit;
    use query::QueryPlan;
    use query::QueryResultRow;
    use std::time::Duration;

    #[test]
    fn actual_query_stream_schema_supports_numeric_proxies() {
        let service = actual_rpc::machine_daemon_rpc_service_descriptor();
        let method = service
            .methods
            .iter()
            .find(|m| m.method_name == "query_stream")
            .unwrap();
        assert_eq!(method.id, vox_types::MethodId(6665204681412186470));
        let plan = vox_types::SchemaSendTracker::plan_for_method_args(method).unwrap();
        println!(
            "query_stream method_id={:?}, schema payload={} bytes",
            method.id,
            plan.to_payload().0.len()
        );
        for (name, shape) in [
            ("QueryPlan", QueryPlan::SHAPE),
            ("QueryLimit", QueryLimit::SHAPE),
            (
                "FacetSafeNonZeroUsizeOption",
                teamy_mft::domain::FacetSafeNonZeroUsizeOption::SHAPE,
            ),
        ] {
            let plan = vox_types::SchemaSendTracker::plan_for_shape(shape).unwrap();
            println!("{name}: schema OK ({} bytes)", plan.to_payload().0.len());
        }
    }

    #[derive(Clone)]
    struct Service;

    impl MachineDaemonRpc for Service {
        async fn ping(&self, _: vox::Tx<DaemonLogWireEvent>) -> Result<PingResponse, MachineError> {
            unreachable!()
        }
        async fn shutdown(&self, _: vox::Tx<DaemonLogWireEvent>) -> Result<(), MachineError> {
            unreachable!()
        }
        async fn query(
            &self,
            _: QueryPlan,
            _: vox::Tx<DaemonLogWireEvent>,
        ) -> Result<QueryResponse, MachineError> {
            unreachable!()
        }
        async fn sync(
            &self,
            _: sync::SyncPlan,
            _: vox::Tx<DaemonLogWireEvent>,
        ) -> Result<(), MachineError> {
            unreachable!()
        }
        async fn status(
            &self,
            _: StatusRequest,
            _: vox::Tx<DaemonLogWireEvent>,
        ) -> Result<StatusResponse, MachineError> {
            unreachable!()
        }
        async fn query_usn_journal(
            &self,
            _: UsnJournalRequest,
            _: vox::Tx<DaemonLogWireEvent>,
        ) -> Result<UsnJournalStatus, MachineError> {
            unreachable!()
        }
        async fn stream_logs(
            &self,
            _: LogStreamRequest,
            _: vox::Tx<DaemonLogWireEvent>,
            _: vox::Rx<u8>,
        ) -> Result<(), MachineError> {
            unreachable!()
        }

        async fn query_stream(
            &self,
            plan: QueryPlan,
            rows: vox::Tx<QueryResultRow>,
            logs: vox::Tx<DaemonLogWireEvent>,
            mut cancel: vox::Rx<u8>,
        ) -> Result<CorrelationId, MachineError> {
            assert_eq!(plan, sample_plan());
            rows.send(sample_row()).await.unwrap();
            rows.close(Vec::new().into()).await.unwrap();
            logs.send(sample_log()).await.unwrap();
            logs.close(Vec::new().into()).await.unwrap();
            assert_eq!(*cancel.recv().await.unwrap().unwrap().get(), 7);
            Ok(correlation_id())
        }
    }

    fn sample_plan() -> QueryPlan {
        let mut plan = QueryPlan::new("<CACHEDIR.TAG>");
        plan.limit = 42.into();
        plan.r#in = vec![r"C:\qualified".into()];
        plan.profile = Some("integration".into());
        plan
    }

    fn sample_row() -> QueryResultRow {
        QueryResultRow {
            path: r"C:\qualified\CACHEDIR.TAG".to_owned().into(),
            has_deleted_entries: false,
            is_filtered: true,
        }
    }

    fn correlation_id() -> CorrelationId {
        "22222222-2222-2222-2222-222222222222".parse().unwrap()
    }

    fn sample_log() -> DaemonLogWireEvent {
        DaemonLogWireEvent {
            timestamp_unix_ms: 1,
            level: DaemonLogLevel::Info,
            target: "phon_proxy_probe".into(),
            file: None,
            line: None,
            message: "query_stream proxy roundtrip".into(),
            request_id: 0,
            method: "query_stream".into(),
            correlation_id: Some(correlation_id().to_string()),
            spans: vec![],
            fields: vec![],
        }
    }

    #[tokio::test]
    async fn actual_query_stream_roundtrips_plan_rows_logs_cancel_and_return() {
        tokio::time::timeout(Duration::from_secs(10), async {
            let (client_link, server_link) = vox::memory_link_pair(16);
            let server = tokio::spawn(async move {
                vox::acceptor_on(server_link)
                    .on_lane(MachineDaemonRpcDispatcher::new(Service))
                    .establish_connection()
                    .await
                    .unwrap()
            });
            let client = vox::initiator_on(client_link)
                .establish::<MachineDaemonRpcClient>()
                .await
                .unwrap();
            let _guard = server.await.unwrap();
            let (rows_tx, mut rows_rx) = vox::channel::<QueryResultRow>();
            let (logs_tx, mut logs_rx) = vox::channel::<DaemonLogWireEvent>();
            let (cancel_tx, cancel_rx) = vox::channel::<u8>();
            let rows = tokio::spawn(async move {
                let row = rows_rx.recv().await.unwrap().unwrap().get().clone();
                assert!(rows_rx.recv().await.unwrap().is_none());
                row
            });
            let logs = tokio::spawn(async move {
                let log = logs_rx.recv().await.unwrap().unwrap().get().clone();
                assert!(logs_rx.recv().await.unwrap().is_none());
                log
            });
            let cancel = tokio::spawn(async move {
                cancel_tx.send(7).await.unwrap();
            });
            let result = client
                .query_stream(sample_plan(), rows_tx, logs_tx, cancel_rx)
                .await
                .unwrap();
            assert_eq!(result, correlation_id());
            assert_eq!(rows.await.unwrap(), sample_row());
            assert_eq!(logs.await.unwrap(), sample_log());
            cancel.await.unwrap();
        })
        .await
        .expect("query_stream should complete without daemon/index access");
    }
}
