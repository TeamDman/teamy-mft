pub mod cli;

pub mod logging_init;
pub mod mft;
pub mod ntfs;
pub mod paths;
pub mod query;
pub mod read;
pub mod search_index;
pub mod sync_dir;

use crate::cli::Cli;
use teamy_windows::console::console_attach;
use tracing::debug;
#[cfg(feature = "tracy")]
use tracing::info_span;

/// Version string combining package version and git revision.
const VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (rev ",
    env!("GIT_REVISION"),
    ")"
);

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
pub fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    #[cfg(windows)]
    {
        // This can fail when stdout/stderr are redirected, so keep startup permissive.
        let _ = teamy_windows::console::enable_ansi_support();

        teamy_windows::string::warn_if_utf8_not_enabled();
    };

    let cli: Cli = figue::Driver::new(
        figue::builder::<Cli>()
            .expect("schema should be valid")
            .cli(move |cli| cli.args_os(std::env::args_os().skip(1)).strict())
            .help(move |help| {
                help.version(VERSION)
                    .include_implementation_source_file(true)
                    .include_implementation_git_url("TeamDman/teamy-mft", env!("GIT_REVISION"))
            })
            .build(),
    )
    .run()
    .unwrap();

    // Initialize logging
    logging_init::init_logging(&cli.global_args)?;

    if let Some(pid) = cli.global_args.console_pid {
        console_attach(pid)?;
    }

    tracy_capture_padding("before_cli_invoke");
    cli.invoke()?;
    tracy_capture_padding("after_cli_invoke");

    debug!("Goodbye!");
    #[cfg(feature = "tracy")]
    debug!(
        "Tracy may take a while to finish sending the profile, during this time the clock will stop in tracy-capture."
    );
    Ok(())
}
