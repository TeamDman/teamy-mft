//! Count a small set of software marker files using one reused published-index
//! query session.
//!
//! repo[impl examples.rs-files]
//!
//! This mirrors the repeated-query pattern used by Cloud Terrastodon's
//! `software list` flow: create one `QuerySession::in_current_process()` and
//! reuse it across multiple count queries instead of reopening the published
//! index cache for each lookup.
//!
//! # Usage
//!
//! ```bash
//! cargo run --example query_software_counts
//! ```
//!
//! Press `Ctrl+C` to stop after the current query returns its best-effort
//! partial count.
//!
//! Requires a synced MFT index. Run `teamy-mft sync` first if needed.

use teamy_mft::cancellation::install_ctrlc_handler;
use teamy_mft::query::ControlFlow;
use teamy_mft::query::QueryNeedle;
use teamy_mft::query::QueryPlan;
use teamy_mft::query::QueryRule;
use teamy_mft::query::QuerySession;

const SOFTWARE_TERMINAL_SEGMENTS: [&str; 3] = [".git", "package.json", "Cargo.toml"];

fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    let cancel = install_ctrlc_handler()?;

    // Reuse one explicit published-index session so repeated queries can keep
    // drive cache state warm in-process.
    let mut session = QuerySession::local()?;

    println!("{:<20} count", "name");
    for segment in SOFTWARE_TERMINAL_SEGMENTS {
        let mut count = 0_usize;
        session.visit_rows(
            QueryPlan::single_rule(QueryRule::EqualsCaseInsensitive(QueryNeedle::new(segment))),
            &cancel,
            |_row| {
                count += 1;
                Ok(ControlFlow::Continue(()))
            },
        )?;
        cancel.bail_if_cancelled()?;
        println!("{:<20} {}", segment, count);
    }

    Ok(())
}
