use humansize::BINARY;
use humansize::format_size;
use std::time::Duration;
use std::time::Instant;
use thousands::Separable;
use tracing::info;

#[derive(Debug)]
pub struct SyncPhase {
    phase: &'static str,
    drive: String,
    start: Instant,
}

impl SyncPhase {
    pub fn start(phase: &'static str, drive_letter: Option<char>) -> Self {
        let drive = match drive_letter {
            Some(drive_letter) => drive_letter.to_string(),
            None => String::from("all"),
        };
        info!(
            phase,
            drive = %drive,
            "Starting sync phase"
        );
        Self {
            phase,
            drive,
            start: Instant::now(),
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    pub fn name(&self) -> &'static str {
        self.phase
    }

    pub fn drive(&self) -> &str {
        &self.drive
    }
}

pub fn elapsed_ms(elapsed: Duration) -> u64 {
    u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
}

pub fn elapsed_human(elapsed: Duration) -> String {
    humantime::format_duration(elapsed).to_string()
}

pub fn u64_from_usize(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

pub fn bytes_human(bytes: u64) -> String {
    format_size(bytes, BINARY)
}

pub fn bytes_per_second(bytes: u64, elapsed: Duration) -> u64 {
    units_per_second(bytes, elapsed)
}

pub fn bytes_per_second_human(bytes: u64, elapsed: Duration) -> String {
    format!("{}/s", bytes_human(bytes_per_second(bytes, elapsed)))
}

pub fn count_per_second(count: usize, elapsed: Duration) -> u64 {
    units_per_second(u64_from_usize(count), elapsed)
}

pub fn count_per_second_human(count: usize, elapsed: Duration) -> String {
    format!(
        "{}/s",
        count_per_second(count, elapsed).separate_with_commas()
    )
}

fn units_per_second(units: u64, elapsed: Duration) -> u64 {
    let elapsed_nanos = elapsed.as_nanos();
    if elapsed_nanos == 0 {
        return 0;
    }

    let per_second = u128::from(units).saturating_mul(1_000_000_000) / elapsed_nanos;
    u64::try_from(per_second).unwrap_or(u64::MAX)
}
