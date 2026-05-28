#![allow(
    clippy::as_pointer_underscore,
    clippy::borrow_as_ptr,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::doc_markdown,
    clippy::default_trait_access,
    clippy::explicit_deref_methods,
    clippy::map_err_ignore,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::ptr_as_ptr,
    clippy::ref_as_ptr,
    clippy::semicolon_outside_block,
    clippy::single_match_else,
    clippy::undocumented_unsafe_blocks,
    clippy::unnecessary_semicolon,
    clippy::uninlined_format_args,
    clippy::wildcard_imports,
    reason = "Vendored Windows utility wrappers intentionally stay close to upstream shape"
)]

pub mod console;
pub mod ctrl_c;
pub mod elevation;
pub mod event_loop;
pub mod handle;
pub mod hicon;
pub mod invocation;
pub mod log;
pub mod module;
pub mod storage;
pub mod string;
pub mod tray;
pub mod window;
