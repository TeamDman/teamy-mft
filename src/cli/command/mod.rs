pub mod fsutil;
pub mod ignore;
pub mod install;
pub mod list_paths;
pub mod protection;
pub mod query;
pub mod service;
pub mod status;
pub mod sync;
pub mod tray;
pub mod uninstall;

mod command_cli;

pub use command_cli::Command;
