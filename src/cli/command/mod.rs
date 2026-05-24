pub mod daemon;
pub mod get_sync_dir;
pub mod ignore;
pub mod install;
pub mod list_paths;
pub mod query;
pub mod set_sync_dir;
pub mod status;
pub mod sync;
pub mod uninstall;

mod command_cli;

pub use command_cli::Command;
