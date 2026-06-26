pub use teamy_cancellation::CancellationToken;
pub use teamy_cancellation::CtrlCHandler;
pub use teamy_cancellation::StopAfterLayer;

/// Create the process cancellation token and install the process-wide Ctrl+C handler.
///
/// # Errors
///
/// Returns an error if the platform handler cannot be registered.
pub fn install_ctrlc_handler() -> eyre::Result<CancellationToken> {
    CtrlCHandler::default().install_with_on_cancel_request(|_reason, _was_first| {
        crate::windows_utils::ctrl_c::request_interrupt_from_process_handler();
    })
}
