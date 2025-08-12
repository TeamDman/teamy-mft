use clap::Parser;
use clap::Subcommand;
use color_eyre::eyre::Result;
use std::path::PathBuf;

mod sync_dir;
pub mod paths;

#[derive(Parser, Debug)]
#[command(
    name = "teamy-mft",
    version,
    about = "Teamy MFT toolkit",
    propagate_version = true
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Get the currently configured sync directory
    GetSyncDir,
    /// Set the sync directory (defaults to current directory if omitted)
    SetSyncDir { path: Option<PathBuf> },
}

fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();

    match cli.command {
        Commands::GetSyncDir => match sync_dir::get_sync_dir()? {
            Some(p) => println!("{}", p.display()),
            None => println!("<not set>"),
        },
        Commands::SetSyncDir { path } => {
            let target = path.unwrap_or(std::env::current_dir()?);
            sync_dir::set_sync_dir(target.clone())?;
            println!("Set sync dir to {}", target.display());
        }
    }

    Ok(())
}
