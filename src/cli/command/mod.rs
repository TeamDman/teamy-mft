pub mod daemon;
pub mod ignore;
pub mod install;
pub mod list_paths;
pub mod query;
pub mod status;
pub mod sync;
pub mod tray;
pub mod uninstall;

mod command_cli;

pub use command_cli::Command;
