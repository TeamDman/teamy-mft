pub mod cli;

pub mod logging_init;
pub mod mft;
pub mod mft_process;
pub mod ntfs;
pub mod paths;
pub mod read;
pub mod search_index;
pub mod sync_dir;

use crate::cli::Cli;
use clap::CommandFactory;
use clap::FromArgMatches;
use teamy_windows::console::console_attach;

/// Entrypoint for the program to reduce coupling to the name of this crate.
///
/// # Errors
///
/// Returns an error if CLI parsing or command execution fails.
pub fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    let cli = Cli::command();
    let cli = Cli::from_arg_matches(&cli.get_matches())?;

    // Initialize logging
    logging_init::init_logging(&cli.global_args)?;

    if let Some(pid) = cli.global_args.console_pid {
        console_attach(pid)?;
    }

    cli.invoke()?;
    Ok(())
}
