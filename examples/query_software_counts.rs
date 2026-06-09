//! Count a small set of software marker files using one reused published-index
//! query session.
//!
//! repo[impl examples.rs-files]
//!
//! This mirrors the repeated-query pattern used by Cloud Terrastodon's
//! `software list` flow: create one `QuerySession::published_index_only()` and
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

use color_eyre::owo_colors::OwoColorize;
use teamy_mft::query::ControlFlow;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use teamy_mft::query::QueryNeedle;
use teamy_mft::query::QueryPlan;
use teamy_mft::query::QueryRule;
use teamy_mft::query::QuerySession;

const SOFTWARE_TERMINAL_SEGMENTS: [&str; 3] = [".git", "package.json", "Cargo.toml"];

fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    let cancel = Arc::new(AtomicBool::new(false));
    {
        // spawn ctrl+c handler
        let cancel = Arc::clone(&cancel);
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("ctrl+c runtime should build");
            let _ = runtime.block_on(tokio::signal::ctrl_c());
            cancel.store(true, Ordering::Relaxed);
            println!("{}", "^C".red().bold());
        });
    };

    // Reuse one explicit published-index session so repeated queries can keep
    // drive cache state warm in-process.
    let mut session = QuerySession::published_index_only()?;

    println!("{:<20} count", "name");
    for segment in SOFTWARE_TERMINAL_SEGMENTS {
        let mut count = 0_usize;
        session.visit_rows_with_cancel(
            QueryPlan::single_rule(QueryRule::EqualsCaseInsensitive(QueryNeedle::new(segment))),
            Some(&cancel),
            |_row| {
                count += 1;
                Ok(ControlFlow::Continue)
            },
        )?;
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        println!("{:<20} {}", segment, count);
    }

    Ok(())
}
