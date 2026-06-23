pub mod cancellation;
pub mod cli;

pub mod daemon;
pub mod domain;
pub mod logging_init;
pub mod machine;
pub mod mft;
pub mod ntfs;
pub mod paths;
pub mod presentation;
pub mod query;
pub mod read;
pub mod search_index;
pub mod status;
pub mod sync;
pub mod tray;
pub mod windows_utils;

use crate::cli::Cli;
use crate::windows_utils::console::console_attach;
use chrono::{DateTime, Local, Utc};
use tracing::debug;
#[cfg(feature = "tracy")]
use tracing::info_span;

// tool[impl cli.version.includes-semver]
// tool[impl cli.version.includes-git-revision]
/// Version string combining package version and git revision.
pub const APP_SEMVER: &str = env!("CARGO_PKG_VERSION");
pub const APP_GIT_REVISION: &str = env!("GIT_REVISION");
pub const APP_BUILD_UNIX_MS: &str = env!("BUILD_UNIX_MS");
pub const DAEMON_RPC_COMPAT_VERSION: u32 = 8;

fn version() -> String {
    let built_at = APP_BUILD_UNIX_MS
        .parse::<i64>()
        .ok()
        .and_then(DateTime::<Utc>::from_timestamp_millis)
        .map_or_else(
            || String::from("unknown build time"),
            |timestamp| {
                timestamp
                    .with_timezone(&Local)
                    .format("%Y-%m-%d %H:%M:%S %Z")
                    .to_string()
            },
        );

    format!("{APP_SEMVER} (rev {APP_GIT_REVISION}, built {built_at})")
}

#[cfg(feature = "tracy")]
fn tracy_capture_padding(phase: &'static str) {
    info_span!("tracy_capture_padding", phase).in_scope(|| {
        std::thread::sleep(std::time::Duration::from_secs(1));
    });
}

#[cfg(not(feature = "tracy"))]
fn tracy_capture_padding(_phase: &'static str) {}

/// Entrypoint for the program to reduce coupling to the name of this crate.
///
/// # Errors
///
/// Returns an error if CLI parsing or command execution fails.
///
/// # Panics
///
/// Panics if the CLI schema is invalid (should never happen with correct code).
// tool[impl cli.help.position-independent]
pub fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    #[cfg(windows)]
    {
        // This can fail when stdout/stderr are redirected, so keep startup permissive.
        let _ = crate::windows_utils::console::enable_ansi_support();

        crate::windows_utils::string::warn_if_utf8_not_enabled();
    };

    // cli[impl parser.args-consistent]
    // cli[impl parser.roundtrip]
    let version = version();
    let cancellation_token = crate::cancellation::install_ctrlc_handler()?;
    let cli: Cli = figue::Driver::new(
        figue::builder::<Cli>()
            .expect("schema should be valid")
            .cli(move |cli| cli.args_os(std::env::args_os().skip(1)).strict())
            .help(move |help| {
                help.version(version)
                    .include_implementation_source_file(true)
                    .include_implementation_git_url("TeamDman/teamy-mft", env!("GIT_REVISION"))
            })
            .build(),
    )
    .run()
    .unwrap();

    // Initialize logging
    logging_init::init_logging(&cli.global_args, cancellation_token.clone())?;

    if let Some(pid) = cli.global_args.console_pid {
        console_attach(pid)?;
    }

    tracy_capture_padding("before_cli_invoke");
    cli.invoke()?;
    tracy_capture_padding("after_cli_invoke");
    cancellation_token.bail_if_cancelled()?;

    debug!("Goodbye!");
    #[cfg(feature = "tracy")]
    debug!(
        "Tracy may take a while to finish sending the profile, during this time the clock will stop in tracy-capture."
    );
    Ok(())
}
