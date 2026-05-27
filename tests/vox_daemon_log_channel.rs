use std::time::Duration;
use teamy_mft::daemon::CorrelationId;
use teamy_mft::daemon::DaemonLogEvent;
use teamy_mft::daemon::DaemonLogField;
use teamy_mft::daemon::DaemonLogLevel;
use teamy_mft::daemon::DaemonLogSpan;
use teamy_mft::daemon::DaemonLogWireEvent;

#[vox::service]
trait LogProbe {
    async fn ping(&self, logs: vox::Tx<DaemonLogWireEvent>) -> String;
}

#[derive(Clone)]
struct LogProbeService;

impl LogProbe for LogProbeService {
    async fn ping(&self, logs: vox::Tx<DaemonLogWireEvent>) -> String {
        logs.send(DaemonLogWireEvent::from(&sample_log_event()))
            .await
            .expect("server should send daemon log event");
        logs.close(Vec::default())
            .await
            .expect("server should close daemon log channel");
        String::from("pong")
    }
}

fn sample_log_event() -> DaemonLogEvent {
    DaemonLogEvent {
        timestamp_unix_ms: 1,
        level: DaemonLogLevel::Info,
        target: String::from("teamy_mft::test"),
        file: Some(String::from("src/machine/daemon.rs")),
        line: Some(399),
        message: String::from("Daemon pong"),
        request_id: 0,
        method: String::from("ping"),
        correlation_id: Some(
            "22222222-2222-2222-2222-222222222222"
                .parse::<CorrelationId>()
                .expect("uuid should parse"),
        ),
        spans: vec![DaemonLogSpan {
            name: String::from("daemon_rpc"),
            target: String::from("teamy_mft::machine::daemon"),
            file: Some(String::from("src/machine/daemon.rs")),
            line: Some(393),
        }],
        fields: vec![DaemonLogField {
            key: String::from("service_name"),
            value: String::from("teamy-mft-daemon"),
        }],
    }
}

async fn pair() -> (LogProbeClient, vox::NoopClient) {
    let (client_link, server_link) = vox::memory_link_pair(16);
    let server = tokio::spawn(async move {
        vox::acceptor_on(server_link)
            .on_connection(LogProbeDispatcher::new(LogProbeService))
            .establish::<vox::NoopClient>()
            .await
            .expect("server establish")
    });
    let client = vox::initiator_on(client_link, vox::TransportMode::Bare)
        .establish::<LogProbeClient>()
        .await
        .expect("client establish");
    let server_guard = server.await.expect("server task");
    (client, server_guard)
}

async fn collect_logs(mut logs_rx: vox::Rx<DaemonLogWireEvent>) -> Vec<DaemonLogEvent> {
    let mut logs = Vec::new();
    while let Ok(Some(event)) = logs_rx.recv().await {
        logs.push(
            DaemonLogEvent::try_from(event.get().clone())
                .expect("wire daemon log event should decode"),
        );
    }
    logs
}

#[tokio::test]
async fn tx_arg_delivers_daemon_log_when_receiver_is_draining_during_call() {
    let (client, _server) = pair().await;
    let (logs_tx, logs_rx) = vox::channel::<DaemonLogWireEvent>();
    let logs = tokio::spawn(collect_logs(logs_rx));

    let response = tokio::time::timeout(Duration::from_secs(5), client.ping(logs_tx))
        .await
        .expect("ping should return")
        .expect("ping should succeed");
    let logs = tokio::time::timeout(Duration::from_secs(5), logs)
        .await
        .expect("log collector should finish")
        .expect("log collector task should not panic");

    assert_eq!(response, "pong");
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].message, "Daemon pong");
    assert_eq!(logs[0].file.as_deref(), Some("src/machine/daemon.rs"));
    assert_eq!(logs[0].spans[0].name, "daemon_rpc");
}

#[tokio::test]
async fn tx_arg_delivers_daemon_log_when_receiver_starts_after_call() {
    let (client, _server) = pair().await;
    let (logs_tx, logs_rx) = vox::channel::<DaemonLogWireEvent>();

    let response = tokio::time::timeout(Duration::from_secs(5), client.ping(logs_tx))
        .await
        .expect("ping should return")
        .expect("ping should succeed");
    let logs = tokio::time::timeout(Duration::from_secs(5), collect_logs(logs_rx))
        .await
        .expect("log collection should finish");

    assert_eq!(response, "pong");
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].message, "Daemon pong");
    assert_eq!(logs[0].file.as_deref(), Some("src/machine/daemon.rs"));
    assert_eq!(logs[0].spans[0].name, "daemon_rpc");
}
