use crate::cancellation::CancellationToken;
use crate::machine::config::published_drive_paths;
use crate::machine::live_drive_state::LiveDriveState;
use crate::machine::live_drive_state::ObservedUsnEvent;
use crate::query::QueryScope;
use crate::query::resolve_query_scopes;
use crate::windows_utils::elevation::ensure_elevated;
use crate::windows_utils::storage::DriveLetterPattern;
use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};
use std::time::Duration;
use tracing::debug;
use tracing::info;
use tracing::warn;

#[derive(Facet, Arbitrary, PartialEq, Debug)]
#[facet(rename_all = "kebab-case")]
#[derive(Default)]
pub struct ServiceWatchUsnArgs {
    /// Drive letter pattern to watch (e.g., `*`, `C`, `CD`, `C,D`). Compatibility alias: `--drive`.
    #[facet(args::named, args::alias = "drive", default)]
    pub drive_letter_pattern: DriveLetterPattern,

    /// Restrict logged events to these paths. Directories include descendants; files match exactly. Repeat `--in` to OR scopes.
    #[facet(args::named, default)]
    pub r#in: Vec<String>,

    /// Poll interval in milliseconds
    #[facet(args::named, default)]
    pub poll_ms: Option<u64>,
}

impl ServiceWatchUsnArgs {
    /// # Errors
    ///
    /// Returns an error if the machine cache cannot be loaded, selected drives
    /// cannot be resolved, USN journals cannot be read, or scope paths cannot be
    /// resolved.
    pub fn invoke(self, cancel: &CancellationToken) -> eyre::Result<()> {
        ensure_elevated()?;
        cancel.bail_if_cancelled()?;
        let config = crate::machine::config::load_required_machine_config()?;
        let scopes = resolve_query_scopes(&self.r#in)?;
        let drive_letters = self
            .drive_letter_pattern
            .into_drive_letters_for_scope_roots(scopes.iter().map(|scope| scope.root.as_path()))?;
        let poll_interval = Duration::from_millis(self.poll_ms.unwrap_or(500).max(50));
        let mut states = Vec::new();

        for drive_letter in drive_letters {
            let paths = published_drive_paths(&config.sync_dir, drive_letter);
            let state =
                LiveDriveState::load_for_observation_with_cancel(&config.sync_dir, paths, cancel)?;
            println!("usn-watch-drive-ready={drive_letter}");
            println!(
                "usn-watch-drive-start-usn-{}={}",
                drive_letter,
                state.current_next_usn()
            );
            states.push(state);
        }

        info!(
            drive_count = states.len(),
            scope_count = scopes.len(),
            poll_ms = poll_interval.as_millis(),
            "Watching USN journal topology events"
        );
        warn!("USN watch starts at the current journal tail and observes new topology events only");
        println!("usn-watch-drive-count={}", states.len());
        println!("usn-watch-scope-count={}", scopes.len());
        println!("usn-watch-poll-ms={}", poll_interval.as_millis());

        while !cancel.is_cancelled() {
            for state in &mut states {
                cancel.bail_if_cancelled()?;
                for event in state.observe_usn_events_with_cancel(cancel)? {
                    cancel.bail_if_cancelled()?;
                    if !event_matches_scopes(&event, &scopes) {
                        debug!(
                            drive = %event.drive_letter,
                            usn = event.usn,
                            name = %event.name,
                            projected_paths = %event.projected_paths.iter().map(|path| path.display().to_string()).collect::<Vec<_>>().join("|"),
                            "Observed USN event outside watch scope"
                        );
                        continue;
                    }
                    log_observed_event(&event);
                }
            }
            std::thread::sleep(poll_interval);
        }

        cancel.bail_if_cancelled()
    }
}

fn event_matches_scopes(event: &ObservedUsnEvent, scopes: &[QueryScope]) -> bool {
    scopes.is_empty()
        || event
            .projected_paths
            .iter()
            .any(|path| scopes.iter().any(|scope| scope.matches_path(path)))
}

fn log_observed_event(event: &ObservedUsnEvent) {
    let reasons = event.reason_names.join("|");
    let projected_paths = event
        .projected_paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join("|");
    debug!(
        drive = %event.drive_letter,
        usn = event.usn,
        frn = event.frn,
        parent_frn = event.parent_frn,
        reason = %reasons,
        reason_mask = format_args!("0x{:08x}", event.reason),
        is_directory = event.is_directory,
        name = %event.name,
        projected_paths = %projected_paths,
        "Observed USN topology event"
    );
    println!("usn-event-drive={}", event.drive_letter);
    println!("usn-event-usn={}", event.usn);
    println!("usn-event-frn={}", event.frn);
    println!("usn-event-parent-frn={}", event.parent_frn);
    println!("usn-event-reason={reasons}");
    println!("usn-event-reason-mask=0x{:08x}", event.reason);
    println!("usn-event-is-directory={}", event.is_directory);
    println!("usn-event-name={}", event.name);
    for path in &event.projected_paths {
        println!("usn-event-path={}", path.display());
    }
}
