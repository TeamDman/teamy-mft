use crate::cli::global_args::GlobalArgs;
use chrono::Local;
use color_eyre::owo_colors::OwoColorize;
use eyre::bail;
use std::fmt;
use std::fs::File;
use std::sync::Arc;
use std::sync::Mutex;
use tracing::Event;
use tracing::debug;
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
        self.inner.format_event(ctx, writer, event)
    }
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
