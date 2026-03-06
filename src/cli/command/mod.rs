pub mod get_sync_dir;
pub mod list_paths;
pub mod query;
pub mod set_sync_dir;
pub mod sync;

mod command_cli;

pub use command_cli::Command;
