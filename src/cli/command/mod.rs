pub mod check;
pub mod get_sync_dir;
pub mod list_paths;
pub mod query;
pub mod robocopy_logs_tui;
pub mod set_sync_dir;
pub mod sync;

mod command;

pub use command::Command;
