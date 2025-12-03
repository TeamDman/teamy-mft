#![feature(btree_cursors)]
pub mod cli;
pub mod drive_letter_pattern;
pub mod engine;
pub mod mft;
pub mod mft_process;
pub mod ntfs;
pub mod paths;
pub mod read;
pub mod robocopy;
pub mod sync_dir;

use crate::cli::Cli;
use clap::CommandFactory;
use clap::FromArgMatches;
use teamy_windows::console::console_attach;
use tracing::Level;
use tracing::debug;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::util::SubscriberInitExt;

/// Manually includes [`::bevy::log::DEFAULT_FILTER`] to create an [`EnvFilter`]
/// 
/// https://github.com/tokio-rs/tracing/issues/1181
/// https://github.com/tokio-rs/tracing/issues/2809
pub const DEFAULT_EXTRA_FILTERS: &str = r#"bevy_shader=warn,offset_allocator=warn,bevy_app=info,bevy_render=info,gilrs=info,cosmic_text=info,naga=warn,wgpu=error,wgpu_hal=warn,bevy_skein=trace,bevy_winit::system=info"#;

/// Initialize tracing subscriber with the given log level.
/// In debug builds, include file and line number without timestamp.
/// In release builds, include timestamp and log level.
pub fn init_tracing(level: Level) {
    let builder = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::builder().parse_lossy(format!(
                "{default_log_level},{extras}",
                default_log_level = level,
                extras = match level {
                    Level::DEBUG | Level::TRACE => DEFAULT_EXTRA_FILTERS,
                    _ => "",
                }
            ))
        }))
        .with_writer(std::io::stderr)
        .pretty();
    #[cfg(debug_assertions)]
    let subscriber = builder
        .with_target(true)
        .with_file(true)
        .with_line_number(true)
        .without_time()
        .finish();
    #[cfg(not(debug_assertions))]
    let subscriber = builder.finish();
    if let Err(error) = subscriber.try_init() {
        eprintln!(
            "Failed to initialize tracing subscriber - are you running `cargo test`? If so, multiple test entrypoints may be running from the same process. https://github.com/tokio-rs/console/issues/505 : {error}"
        );
        return;
    }
    debug!("Tracing initialized with level: {:?}", level);
}

// Entrypoint for the program to reduce coupling to the name of this crate.
pub fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    let cli = Cli::command();
    let cli = Cli::from_arg_matches(&cli.get_matches())?;

    if let Some(pid) = cli.global_args.console_pid {
        console_attach(pid)?;
    }

    cli.invoke()?;
    Ok(())
}
