pub mod get_sync_dir;
pub mod list_paths;
pub mod query;
pub mod set_sync_dir;
pub mod sync;

#[allow(
    clippy::module_inception,
    reason = "module structure requires submodule with same name"
)]
mod command;

pub use command::Command;
