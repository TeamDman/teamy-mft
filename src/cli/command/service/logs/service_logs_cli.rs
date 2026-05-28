use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};

/// Replay recent daemon logs and optionally keep streaming new daemon logs.
#[derive(Facet, Arbitrary, PartialEq, Debug, Default)]
#[facet(rename_all = "kebab-case")]
pub struct ServiceLogsArgs {
    /// Keep streaming daemon logs until the command is interrupted.
    #[facet(args::named, args::short = 'f', default)]
    pub follow: bool,

    /// Only show new logs instead of replaying the daemon's recent in-memory buffer first.
    #[facet(args::named, default)]
    pub no_replay: bool,
}

impl ServiceLogsArgs {
    /// # Errors
    ///
    /// Returns an error if the daemon cannot be reached or the log stream cannot be opened.
    pub fn invoke(self) -> eyre::Result<()> {
        let config = crate::machine::ipc::load_machine_daemon_client_config()?;
        let ready_daemon = crate::machine::ipc::ensure_daemon_ready(&config)?;
        crate::machine::ipc::ensure_rpc_compatibility(&ready_daemon.ping)?;

        let (logs_tx, logs_rx) = vox::channel::<crate::machine::daemon_log::DaemonLogWireEvent>();
        let (_cancel_tx, cancel_rx) = vox::channel::<u8>();
        let log_drain = crate::machine::daemon_log::spawn_stderr_log_drain(logs_rx);
        let result = crate::machine::ipc::stream_logs(
            &config,
            crate::machine::ipc::LogStreamRequest {
                replay_recent: !self.no_replay,
                follow: self.follow,
            },
            logs_tx,
            cancel_rx,
        );
        let _ = log_drain.join();
        result
    }
}
