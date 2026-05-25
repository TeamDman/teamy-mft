use std::collections::VecDeque;
use std::fmt;
use std::sync::LazyLock;
use std::sync::Mutex;
use std::time::Duration;
use teamy_mft_daemon_rpc::CorrelationId;
pub use teamy_mft_daemon_rpc::DaemonLogEvent;
pub use teamy_mft_daemon_rpc::DaemonLogField;
pub use teamy_mft_daemon_rpc::DaemonLogLevel;
use tokio::sync::broadcast;
use tokio::sync::oneshot;
use tracing::Event;
use tracing::Id;
use tracing::Subscriber;
use tracing::field::Field;
use tracing::field::Visit;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

static DAEMON_LOG_HUB: LazyLock<DaemonLogHub> = LazyLock::new(|| DaemonLogHub::new(2_048));

#[derive(Debug)]
pub struct DaemonLogHub {
    capacity: usize,
    events: Mutex<VecDeque<DaemonLogEvent>>,
    live_tx: broadcast::Sender<DaemonLogEvent>,
}

impl DaemonLogHub {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let (live_tx, _) = broadcast::channel(capacity.max(1));
        Self {
            capacity,
            events: Mutex::new(VecDeque::with_capacity(capacity)),
            live_tx,
        }
    }

    /// # Panics
    ///
    /// Panics if the in-memory daemon log buffer mutex is poisoned.
    pub fn publish(&self, event: DaemonLogEvent) {
        let mut guard = self
            .events
            .lock()
            .expect("daemon log buffer mutex poisoned");
        if guard.len() == self.capacity {
            let _ = guard.pop_front();
        }
        guard.push_back(event.clone());
        drop(guard);
        let _ = self.live_tx.send(event);
    }

    #[must_use]
    /// # Panics
    ///
    /// Panics if the in-memory daemon log buffer mutex is poisoned.
    pub fn snapshot(&self) -> Vec<DaemonLogEvent> {
        self.events
            .lock()
            .expect("daemon log buffer mutex poisoned")
            .iter()
            .cloned()
            .collect()
    }

    #[must_use]
    /// # Panics
    ///
    /// Panics if the in-memory daemon log buffer mutex is poisoned.
    pub fn len(&self) -> usize {
        self.events
            .lock()
            .expect("daemon log buffer mutex poisoned")
            .len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<DaemonLogEvent> {
        self.live_tx.subscribe()
    }
}

#[must_use]
pub fn daemon_log_hub() -> &'static DaemonLogHub {
    &DAEMON_LOG_HUB
}

#[derive(Debug, Default, Clone)]
struct RouteFields {
    correlation_id: Option<CorrelationId>,
    rpc_method: Option<String>,
}

#[derive(Debug, Default)]
struct TraceFieldVisitor {
    correlation_id: Option<CorrelationId>,
    rpc_method: Option<String>,
    message: Option<String>,
    fields: Vec<DaemonLogField>,
}

impl TraceFieldVisitor {
    fn record_rendered(&mut self, field: &Field, rendered: &str) {
        let value = strip_quotes(rendered);
        match field.name() {
            "correlation_id" => {
                if let Ok(correlation_id) = value.parse::<CorrelationId>() {
                    self.correlation_id = Some(correlation_id);
                } else {
                    self.fields.push(DaemonLogField {
                        key: field.name().to_string(),
                        value,
                    });
                }
            }
            "rpc_method" => self.rpc_method = Some(value),
            "message" => self.message = Some(value),
            _ => self.fields.push(DaemonLogField {
                key: field.name().to_string(),
                value,
            }),
        }
    }

    fn route_fields(&self) -> RouteFields {
        RouteFields {
            correlation_id: self.correlation_id.clone(),
            rpc_method: self.rpc_method.clone(),
        }
    }
}

impl Visit for TraceFieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.record_rendered(field, &format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.record_rendered(field, value);
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.record_rendered(field, &value.to_string());
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.record_rendered(field, &value.to_string());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.record_rendered(field, &value.to_string());
    }

    fn record_i128(&mut self, field: &Field, value: i128) {
        self.record_rendered(field, &value.to_string());
    }

    fn record_u128(&mut self, field: &Field, value: u128) {
        self.record_rendered(field, &value.to_string());
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.record_rendered(field, &value.to_string());
    }
}

fn strip_quotes(value: &str) -> String {
    value.trim_matches('"').to_string()
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DaemonTraceLayer;

impl<S> Layer<S> for DaemonTraceLayer
where
    S: Subscriber + for<'span> LookupSpan<'span>,
{
    fn on_new_span(&self, attrs: &tracing::span::Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id) {
            let mut visitor = TraceFieldVisitor::default();
            attrs.record(&mut visitor);
            let route_fields = visitor.route_fields();
            if route_fields.correlation_id.is_some() || route_fields.rpc_method.is_some() {
                span.extensions_mut().insert(route_fields);
            }
        }
    }

    fn on_record(&self, id: &Id, values: &tracing::span::Record<'_>, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id) {
            let mut visitor = TraceFieldVisitor::default();
            values.record(&mut visitor);
            let route_fields = visitor.route_fields();
            if route_fields.correlation_id.is_some() || route_fields.rpc_method.is_some() {
                span.extensions_mut().insert(route_fields);
            }
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let mut visitor = TraceFieldVisitor::default();
        event.record(&mut visitor);

        let mut correlation_id = visitor.correlation_id.clone();
        let mut rpc_method = visitor.rpc_method.clone();
        if let Some(scope) = ctx.event_scope(event) {
            for span in scope.from_root() {
                if let Some(route_fields) = span.extensions().get::<RouteFields>() {
                    if correlation_id.is_none() {
                        correlation_id.clone_from(&route_fields.correlation_id);
                    }
                    if rpc_method.is_none() {
                        rpc_method.clone_from(&route_fields.rpc_method);
                    }
                }
            }
        }

        let event = DaemonLogEvent {
            timestamp_unix_ms: crate::machine::config::current_unix_ms(),
            level: map_level(*metadata.level()),
            target: metadata.target().to_string(),
            message: visitor
                .message
                .unwrap_or_else(|| metadata.name().to_string()),
            request_id: 0,
            method: rpc_method.unwrap_or_else(|| String::from("global")),
            correlation_id,
            fields: visitor.fields,
        };
        daemon_log_hub().publish(event);
    }
}

fn map_level(level: tracing::Level) -> DaemonLogLevel {
    match level {
        tracing::Level::TRACE => DaemonLogLevel::Trace,
        tracing::Level::DEBUG => DaemonLogLevel::Debug,
        tracing::Level::INFO => DaemonLogLevel::Info,
        tracing::Level::WARN => DaemonLogLevel::Warn,
        tracing::Level::ERROR => DaemonLogLevel::Error,
    }
}

pub struct LogForwarderHandle {
    stop_tx: Option<oneshot::Sender<()>>,
    join_handle: tokio::task::JoinHandle<()>,
}

impl fmt::Debug for LogForwarderHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LogForwarderHandle").finish_non_exhaustive()
    }
}

#[must_use]
pub fn spawn_correlation_log_forwarder(
    correlation_id: CorrelationId,
    logs_tx: vox::Tx<DaemonLogEvent>,
) -> LogForwarderHandle {
    spawn_log_forwarder(Some(correlation_id), logs_tx)
}

#[must_use]
pub fn spawn_global_log_forwarder(logs_tx: vox::Tx<DaemonLogEvent>) -> LogForwarderHandle {
    spawn_log_forwarder(None, logs_tx)
}

fn spawn_log_forwarder(
    correlation_id: Option<CorrelationId>,
    logs_tx: vox::Tx<DaemonLogEvent>,
) -> LogForwarderHandle {
    let mut live_rx = daemon_log_hub().subscribe();
    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
    let join_handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut stop_rx => {
                    drain_available_events(&mut live_rx, correlation_id.as_ref(), &logs_tx).await;
                    break;
                }
                recv_result = live_rx.recv() => {
                    match recv_result {
                        Ok(event) => {
                            if matches_correlation_id(&event, correlation_id.as_ref())
                                && logs_tx.send(event).await.is_err()
                            {
                                break;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                            tracing::warn!(skipped, "Daemon log forwarder lagged behind live events");
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
        }
        let _ = logs_tx.close(Vec::default()).await;
    });
    LogForwarderHandle {
        stop_tx: Some(stop_tx),
        join_handle,
    }
}

async fn drain_available_events(
    live_rx: &mut broadcast::Receiver<DaemonLogEvent>,
    correlation_id: Option<&CorrelationId>,
    logs_tx: &vox::Tx<DaemonLogEvent>,
) {
    loop {
        match live_rx.try_recv() {
            Ok(event) => {
                if matches_correlation_id(&event, correlation_id)
                    && logs_tx.send(event).await.is_err()
                {
                    break;
                }
            }
            Err(
                tokio::sync::broadcast::error::TryRecvError::Empty
                | tokio::sync::broadcast::error::TryRecvError::Closed,
            ) => break,
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {}
        }
    }
}

fn matches_correlation_id(event: &DaemonLogEvent, correlation_id: Option<&CorrelationId>) -> bool {
    match correlation_id {
        Some(correlation_id) => event.correlation_id.as_ref() == Some(correlation_id),
        None => true,
    }
}

pub async fn stop_log_forwarder(mut forwarder: LogForwarderHandle) {
    if let Some(stop_tx) = forwarder.stop_tx.take() {
        let _ = stop_tx.send(());
    }
    let _ = tokio::time::timeout(Duration::from_secs(2), &mut forwarder.join_handle).await;
}

#[must_use]
pub fn render_daemon_log_event(event: &DaemonLogEvent) -> String {
    let fields = event
        .fields
        .iter()
        .map(|field| format!("{}={}", field.key, field.value))
        .collect::<Vec<_>>()
        .join(", ");
    let correlation_id = event
        .correlation_id
        .as_ref()
        .map_or_else(|| String::from("global"), ToString::to_string);
    if fields.is_empty() {
        format!(
            "[daemon:{}:{}:{}] {}",
            event.level, event.method, correlation_id, event.message
        )
    } else {
        format!(
            "[daemon:{}:{}:{}] {} ({fields})",
            event.level, event.method, correlation_id, event.message
        )
    }
}

fn emit_forwarded_daemon_log(event: &DaemonLogEvent) {
    let fields = event
        .fields
        .iter()
        .map(|field| format!("{}={}", field.key, field.value))
        .collect::<Vec<_>>()
        .join(", ");
    match event.level {
        DaemonLogLevel::Trace => tracing::trace!(
            target: "teamy_mft::daemon_remote",
            daemon_target = %event.target,
            rpc_method = %event.method,
            correlation_id = ?event.correlation_id,
            daemon_fields = %fields,
            "{}",
            event.message
        ),
        DaemonLogLevel::Debug => tracing::debug!(
            target: "teamy_mft::daemon_remote",
            daemon_target = %event.target,
            rpc_method = %event.method,
            correlation_id = ?event.correlation_id,
            daemon_fields = %fields,
            "{}",
            event.message
        ),
        DaemonLogLevel::Info => tracing::info!(
            target: "teamy_mft::daemon_remote",
            daemon_target = %event.target,
            rpc_method = %event.method,
            correlation_id = ?event.correlation_id,
            daemon_fields = %fields,
            "{}",
            event.message
        ),
        DaemonLogLevel::Warn => tracing::warn!(
            target: "teamy_mft::daemon_remote",
            daemon_target = %event.target,
            rpc_method = %event.method,
            correlation_id = ?event.correlation_id,
            daemon_fields = %fields,
            "{}",
            event.message
        ),
        DaemonLogLevel::Error => tracing::error!(
            target: "teamy_mft::daemon_remote",
            daemon_target = %event.target,
            rpc_method = %event.method,
            correlation_id = ?event.correlation_id,
            daemon_fields = %fields,
            "{}",
            event.message
        ),
    }
}

#[must_use]
pub fn spawn_stderr_log_drain(mut rx: vox::Rx<DaemonLogEvent>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                eprintln!("Failed building daemon log drain runtime: {error}");
                return;
            }
        };
        runtime.block_on(async move {
            loop {
                match rx.recv().await {
                    Ok(Some(event)) => emit_forwarded_daemon_log(event.get()),
                    Ok(None) => break,
                    Err(error) => {
                        tracing::warn!(error = %error, "Failed draining daemon logs");
                        break;
                    }
                }
            }
        });
    })
}

#[cfg(test)]
mod tests {
    use super::CorrelationId;
    use super::DaemonLogEvent;
    use super::DaemonLogLevel;
    use super::daemon_log_hub;
    use std::str::FromStr;

    #[test]
    fn hub_keeps_only_latest_events() {
        let hub = daemon_log_hub();
        hub.publish(DaemonLogEvent {
            timestamp_unix_ms: 1,
            level: DaemonLogLevel::Info,
            target: String::from("test"),
            message: String::from("event-1"),
            request_id: 0,
            method: String::from("query"),
            correlation_id: Some(
                CorrelationId::from_str("00000000-0000-0000-0000-000000000000")
                    .expect("uuid should parse"),
            ),
            fields: Vec::new(),
        });
        hub.publish(DaemonLogEvent {
            timestamp_unix_ms: 2,
            level: DaemonLogLevel::Info,
            target: String::from("test"),
            message: String::from("event-2"),
            request_id: 0,
            method: String::from("query"),
            correlation_id: Some(
                CorrelationId::from_str("ffffffff-ffff-ffff-ffff-ffffffffffff")
                    .expect("uuid should parse"),
            ),
            fields: Vec::new(),
        });

        let snapshot = hub.snapshot();
        let expected = CorrelationId::from_str("ffffffff-ffff-ffff-ffff-ffffffffffff")
            .expect("uuid should parse");
        assert!(!snapshot.is_empty());
        assert_eq!(
            snapshot
                .last()
                .expect("hub should contain latest event")
                .correlation_id
                .as_ref(),
            Some(&expected)
        );
    }
}
