use crate::cli::global_args::GlobalArgs;
use chrono::Local;
use color_eyre::owo_colors::OwoColorize;
use eyre::bail;
use std::fmt;
use std::fs::File;
use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::Mutex;
use std::time::Instant;
use tracing::Event;
use tracing::debug;
use tracing::field::Field;
use tracing::field::Visit;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::Registry;
use tracing_subscriber::filter::FilterFn;
use tracing_subscriber::fmt::FmtContext;
use tracing_subscriber::fmt::FormatEvent;
use tracing_subscriber::fmt::FormatFields;
#[cfg(all(feature = "tracy", not(test)))]
use tracing_subscriber::fmt::format::DefaultFields;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::writer::BoxMakeWriter;
use tracing_subscriber::fmt::writer::MakeWriterExt;
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;

struct SourceAwareEventFormat<E> {
    inner: E,
}

static LOG_START: LazyLock<Instant> = LazyLock::new(Instant::now);

#[derive(Debug, Default)]
struct DaemonRemotePrettyFields {
    message: Option<String>,
    target: Option<String>,
    file: Option<String>,
    line: Option<String>,
    spans: Option<String>,
    rpc_method: Option<String>,
    correlation_id: Option<String>,
    fields: Option<String>,
}

impl DaemonRemotePrettyFields {
    fn record_rendered(&mut self, field: &Field, rendered: &str) {
        let value = rendered.trim_matches('"').to_string();
        match field.name() {
            "message" => self.message = Some(value),
            "daemon_target" => self.target = Some(value),
            "daemon_file" => self.file = Some(value),
            "daemon_line" => self.line = Some(value),
            "daemon_spans" => self.spans = Some(value),
            "rpc_method" => self.rpc_method = Some(value),
            "correlation_id" => self.correlation_id = Some(value),
            "daemon_fields" => self.fields = Some(value),
            _ => {}
        }
    }
}

impl Visit for DaemonRemotePrettyFields {
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

#[cfg(all(feature = "tracy", not(test)))]
#[derive(Default)]
struct TracyLayerConfig {
    formatter: TracyPlainFields,
}

#[cfg(all(feature = "tracy", not(test)))]
impl tracing_tracy::Config for TracyLayerConfig {
    type Formatter = TracyPlainFields;

    fn formatter(&self) -> &Self::Formatter {
        &self.formatter
    }
}

#[cfg(all(feature = "tracy", not(test)))]
#[derive(Default)]
struct TracyPlainFields(DefaultFields);

#[cfg(all(feature = "tracy", not(test)))]
impl<'writer> FormatFields<'writer> for TracyPlainFields {
    fn format_fields<R: tracing_subscriber::field::RecordFields>(
        &self,
        writer: Writer<'writer>,
        fields: R,
    ) -> fmt::Result {
        self.0.format_fields(writer, fields)
    }
}

impl<S, N, E> FormatEvent<S, N> for SourceAwareEventFormat<E>
where
    S: tracing::Subscriber + for<'span> LookupSpan<'span>,
    N: for<'writer> FormatFields<'writer> + 'static,
    E: FormatEvent<S, N>,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let source = if event
            .metadata()
            .target()
            .starts_with(crate::machine::daemon_log::DAEMON_REMOTE_TARGET)
        {
            if writer.has_ansi_escapes() {
                "[daemon] ".bright_blue().to_string()
            } else {
                String::from("[daemon] ")
            }
        } else {
            if writer.has_ansi_escapes() {
                "[client] ".bright_green().to_string()
            } else {
                String::from("[client] ")
            }
        };
        writer.write_str(&source)?;
        if event.metadata().target() == crate::machine::daemon_log::DAEMON_REMOTE_TARGET {
            return format_daemon_remote_event(&mut writer, event);
        }
        self.inner.format_event(ctx, writer, event)
    }
}

fn format_daemon_remote_event(writer: &mut Writer<'_>, event: &Event<'_>) -> fmt::Result {
    let mut fields = DaemonRemotePrettyFields::default();
    event.record(&mut fields);

    let ansi = writer.has_ansi_escapes();
    let elapsed = LOG_START.elapsed().as_secs_f64();
    let level = event.metadata().level();
    let target = fields
        .target
        .as_deref()
        .unwrap_or_else(|| event.metadata().target());
    let message = fields
        .message
        .as_deref()
        .unwrap_or_else(|| event.metadata().name());

    write!(
        writer,
        "{} {} {}{} {}",
        style_dim(ansi, &format!("{elapsed:>16.9}s")),
        style_level(ansi, level, &level.to_string()),
        style_level_bold(ansi, level, target),
        style_level(ansi, level, ":"),
        style_level(ansi, level, message)
    )?;
    if let Some(event_fields) = fields.fields.as_deref().filter(|fields| !fields.is_empty()) {
        write!(
            writer,
            "{} {}",
            style_level(ansi, level, ","),
            style_daemon_field_list(ansi, level, event_fields)
        )?;
    }

    match (
        fields.file.as_deref().filter(|file| *file != "unknown"),
        fields.line.as_deref().filter(|line| *line != "unknown"),
    ) {
        (Some(file), Some(line)) => write!(
            writer,
            "\n    {} {file}:{line}",
            style_source_label(ansi, "at")
        )?,
        (Some(file), None) => write!(writer, "\n    {} {file}", style_source_label(ansi, "at"))?,
        _ => {}
    }

    if let Some(spans) = fields.spans.as_deref().filter(|spans| !spans.is_empty()) {
        write!(writer, "\n    {} {spans}", style_source_label(ansi, "in"))?;
        let method = fields
            .rpc_method
            .as_deref()
            .filter(|method| *method != "global");
        let correlation_id = fields
            .correlation_id
            .as_deref()
            .filter(|correlation_id| *correlation_id != "global");
        match (method, correlation_id) {
            (Some(method), Some(correlation_id)) => {
                write!(
                    writer,
                    " {} {}{}, {}{}",
                    style_dim(ansi, "with"),
                    style_level_bold(ansi, level, "rpc_method"),
                    style_level(ansi, level, &format!("=\"{method}\"")),
                    style_level_bold(ansi, level, "correlation_id"),
                    style_level(ansi, level, &format!("={correlation_id}"))
                )?;
            }
            (Some(method), None) => write!(
                writer,
                " {} {}{}",
                style_dim(ansi, "with"),
                style_level_bold(ansi, level, "rpc_method"),
                style_level(ansi, level, &format!("=\"{method}\""))
            )?,
            (None, Some(correlation_id)) => write!(
                writer,
                " {} {}{}",
                style_dim(ansi, "with"),
                style_level_bold(ansi, level, "correlation_id"),
                style_level(ansi, level, &format!("={correlation_id}"))
            )?,
            (None, None) => {}
        }
    }

    writeln!(writer)
}

fn style_level(ansi: bool, level: &tracing::Level, value: &str) -> String {
    if !ansi {
        return value.to_string();
    }

    match *level {
        tracing::Level::TRACE => value.purple().to_string(),
        tracing::Level::DEBUG => value.blue().to_string(),
        tracing::Level::INFO => value.green().to_string(),
        tracing::Level::WARN => value.yellow().to_string(),
        tracing::Level::ERROR => value.red().to_string(),
    }
}

fn style_level_bold(ansi: bool, level: &tracing::Level, value: &str) -> String {
    if !ansi {
        return value.to_string();
    }

    match *level {
        tracing::Level::TRACE => value.purple().bold().to_string(),
        tracing::Level::DEBUG => value.blue().bold().to_string(),
        tracing::Level::INFO => value.green().bold().to_string(),
        tracing::Level::WARN => value.yellow().bold().to_string(),
        tracing::Level::ERROR => value.red().bold().to_string(),
    }
}

fn style_dim(ansi: bool, value: &str) -> String {
    if ansi {
        value.dimmed().to_string()
    } else {
        value.to_string()
    }
}

fn style_source_label(ansi: bool, value: &str) -> String {
    if ansi {
        value.dimmed().italic().to_string()
    } else {
        value.to_string()
    }
}

fn style_daemon_field_list(ansi: bool, level: &tracing::Level, fields: &str) -> String {
    fields
        .split(", ")
        .map(|field| {
            let Some((key, value)) = field.split_once('=') else {
                return style_level(ansi, level, field);
            };
            format!(
                "{}{}",
                style_level_bold(ansi, level, key),
                style_level(ansi, level, &format!("={value}"))
            )
        })
        .collect::<Vec<_>>()
        .join(&style_level(ansi, level, ", "))
}

fn default_log_filter(global_args: &GlobalArgs) -> eyre::Result<EnvFilter> {
    if let Some(filter) = global_args.log_filter.as_ref() {
        if global_args.debug {
            bail!("cannot specify log filter with --debug");
        }
        return EnvFilter::builder().parse(filter).map_err(Into::into);
    }

    let own_level = if global_args.debug { "debug" } else { "info" };
    let filter = format!("warn,teamy_mft={own_level}");
    EnvFilter::builder().parse(filter).map_err(Into::into)
}

fn stderr_event_filter(metadata: &tracing::Metadata<'_>) -> bool {
    metadata.target() != crate::machine::daemon_log::DAEMON_REMOTE_SPAN_TRANSITION_TARGET
}

#[cfg(all(feature = "tracy", not(test)))]
fn tracy_log_filter() -> eyre::Result<EnvFilter> {
    EnvFilter::builder()
        .parse(
            [
                "trace",
                "cranelift_codegen=warn",
                "cranelift_frontend=warn",
                "cranelift_jit=warn",
                "cranelift_native=warn",
                "regalloc2=warn",
                "wasmtime=warn",
            ]
            .join(","),
        )
        .map_err(Into::into)
}

// tool[impl logging.stderr-output]
// tool[impl logging.file-path-option]
// tool[impl logging.file-structured-ndjson]
/// Initialize logging based on the provided configuration.
///
/// # Errors
///
/// This function will return an error if creating the log file or directories fails.
///
/// # Panics
///
/// This function may panic if locking or cloning the log file handle fails.
pub fn init_logging(global_args: &GlobalArgs) -> eyre::Result<()> {
    LazyLock::force(&LOG_START);

    let subscriber = Registry::default();

    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_file(cfg!(debug_assertions))
        .with_line_number(cfg!(debug_assertions))
        .with_target(true)
        .with_writer(std::io::stderr.and(crate::windows_utils::log::LOG_BUFFER.clone()))
        .event_format(SourceAwareEventFormat {
            inner: tracing_subscriber::fmt::format()
                .pretty()
                .with_timer(tracing_subscriber::fmt::time::uptime()),
        })
        .with_filter(default_log_filter(global_args)?)
        .with_filter(FilterFn::new(stderr_event_filter));

    let subscriber = subscriber.with(stderr_layer);
    let subscriber = subscriber.with(crate::machine::daemon_log::DaemonTraceLayer);

    let json_log_path = match global_args.log_file.as_ref() {
        None => None,
        Some(path) if std::path::PathBuf::from(path).is_dir() => {
            let timestamp = Local::now().format("%Y-%m-%d_%H-%M-%S");
            let filename = format!("log_{timestamp}.ndjson");
            Some(std::path::PathBuf::from(path).join(filename))
        }
        Some(path) => Some(std::path::PathBuf::from(path)),
    };
    let json_layer = if let Some(ref json_log_path) = json_log_path {
        if let Some(parent) = json_log_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = File::create(json_log_path)?;
        let file = Arc::new(Mutex::new(file));
        let json_writer = BoxMakeWriter::new(move || {
            file.lock()
                .expect("failed to lock json log file")
                .try_clone()
                .expect("failed to clone json log file handle")
        });

        let json_layer = tracing_subscriber::fmt::layer()
            .event_format(tracing_subscriber::fmt::format().json())
            .with_file(true)
            .with_target(false)
            .with_line_number(true)
            .with_writer(json_writer)
            .with_filter(default_log_filter(global_args)?);
        Some(json_layer)
    } else {
        None
    };
    let subscriber = subscriber.with(json_layer);

    #[cfg(all(feature = "tracy", not(test)))]
    let subscriber = subscriber.with(
        tracing_tracy::TracyLayer::new(TracyLayerConfig::default())
            .with_filter(tracy_log_filter()?),
    );

    if let Err(error) = subscriber.try_init() {
        eprintln!(
            "Failed to initialize tracing subscriber - are you running `cargo test`? If so, multiple test entrypoints may be running from the same process. https://github.com/tokio-rs/console/issues/505 : {error}"
        );
        return Ok(());
    }

    #[cfg(all(feature = "tracy", not(test)))]
    tracing::info!(
        "Tracy profiling layer added, memory usage will increase until a client is connected"
    );

    debug!(
        ?json_log_path,
        debug = global_args.debug,
        "Tracing initialized"
    );

    // Because our logging uses uptime as the timestamp, log the current time at startup to provide a reference point for when events occurred.
    tracing::info!("Current time: {}", Local::now().format("%Y-%m-%d %H:%M:%S"));

    Ok(())
}
