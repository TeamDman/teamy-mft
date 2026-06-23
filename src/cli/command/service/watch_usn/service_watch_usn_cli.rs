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
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tracing::debug;
use tracing::info;
use tracing::warn;

#[derive(Facet, Arbitrary, PartialEq, Debug)]
#[facet(rename_all = "kebab-case")]
pub struct ServiceWatchUsnArgs {
    /// Drive letter pattern to watch (e.g., `*`, `C`, `CD`, `C,D`). Compatibility alias: `--drive`.
    #[facet(args::named, args::long_alias = "drive", default)]
    pub drive_letter_pattern: DriveLetterPattern,

    /// Restrict logged events to these paths. Directories include descendants; files match exactly. Repeat `--in` to OR scopes.
    #[facet(args::named, default)]
    pub r#in: Vec<String>,

    /// Poll interval in milliseconds
    #[facet(args::named, default)]
    pub poll_ms: Option<u64>,
}

impl Default for ServiceWatchUsnArgs {
    fn default() -> Self {
        Self {
            drive_letter_pattern: DriveLetterPattern::default(),
            r#in: Vec::new(),
            poll_ms: None,
        }
    }
}

impl ServiceWatchUsnArgs {
    /// # Errors
    ///
    /// Returns an error if the machine cache cannot be loaded, selected drives
    /// cannot be resolved, USN journals cannot be read, or scope paths cannot be
    /// resolved.
    pub fn invoke(self) -> eyre::Result<()> {
        ensure_elevated()?;
        let _cancel_guard = crate::windows_utils::ctrl_c::use_graceful_cancellation();
        let cancel = AtomicBool::new(false);
        let config = crate::machine::config::load_required_machine_config()?;
        let scopes = resolve_query_scopes(&self.r#in)?;
        let drive_letters =
            if self.drive_letter_pattern == DriveLetterPattern::default() && !scopes.is_empty() {
                drive_letters_from_scopes(&scopes)?
            } else {
                self.drive_letter_pattern.into_drive_letters()?
            };
        let poll_interval = Duration::from_millis(self.poll_ms.unwrap_or(500).max(50));
        let mut states = Vec::new();

        for drive_letter in drive_letters {
            let paths = published_drive_paths(&config.sync_dir, drive_letter);
            states.push(LiveDriveState::load_for_observation_with_cancel(
                &config.sync_dir,
                paths,
                Some(&cancel),
            )?);
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

        while !crate::windows_utils::ctrl_c::interrupted() {
            for state in &mut states {
                for event in state.observe_usn_events_with_cancel(Some(&cancel))? {
                    if !event_matches_scopes(&event, &scopes) {
                        continue;
                    }
                    log_observed_event(&event);
                }
            }
            std::thread::sleep(poll_interval);
        }

        cancel.store(true, Ordering::Relaxed);
        Ok(())
    }
}

fn event_matches_scopes(event: &ObservedUsnEvent, scopes: &[QueryScope]) -> bool {
    scopes.is_empty()
        || event
            .projected_paths
            .iter()
            .any(|path| scopes.iter().any(|scope| scope.matches_path(path)))
}

fn drive_letters_from_scopes(scopes: &[QueryScope]) -> eyre::Result<Vec<char>> {
    let mut drive_letters = Vec::new();
    for scope in scopes {
        let Some(prefix) = scope.root.as_path().components().next() else {
            continue;
        };
        let std::path::Component::Prefix(prefix) = prefix else {
            continue;
        };
        let (std::path::Prefix::Disk(drive) | std::path::Prefix::VerbatimDisk(drive)) =
            prefix.kind()
        else {
            continue;
        };
        let drive = char::from(drive).to_ascii_uppercase();
        if !drive_letters.contains(&drive) {
            drive_letters.push(drive);
        }
    }
    eyre::ensure!(
        !drive_letters.is_empty(),
        "Could not infer drive letters from --in scopes; pass --drive explicitly"
    );
    Ok(drive_letters)
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
