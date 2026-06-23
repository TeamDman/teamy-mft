//! Count file extensions for all indexed files under a directory.
//!
//! repo[impl examples.rs-files]
//!
//! Uses the zero-argument `<>` match-all rule together with `--in`-style scope
//! filtering so the query enumerates every indexed path below one directory.
//! The example then filters out directories locally because Teamy MFT indexes
//! both files and directories.
//!
//! # Usage
//!
//! ```bash
//! cargo run --example query_extension_counts -- [directory]
//! ```
//!
//! Defaults to `CARGO_MANIFEST_DIR` when no directory argument is provided.
//!
//! Requires a synced MFT index. Run `teamy-mft sync` first if needed.

use std::collections::BTreeMap;
use std::path::PathBuf;
use teamy_mft::cli::command::query::QueryArgs;
use teamy_mft::cli::global_args::GlobalArgs;
use teamy_mft::logging_init::init_logging;
use teamy_mft::query::ControlFlow;
use teamy_mft::query::QueryPlan;
use teamy_mft::query::QueryRule;

const NO_EXTENSION: &str = "(no extension)";

fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    init_logging(&GlobalArgs::default())?;

    let scope = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    let scope = dunce::canonicalize(&scope)?;

    let mut plan = QueryPlan::single_rule(QueryRule::MatchAll);
    plan.r#in = vec![scope.to_string_lossy().into_owned()];
    let args = QueryArgs {
        plan,
        ..Default::default()
    };

    let mut raw_counts = BTreeMap::<String, usize>::new();
    let mut total_files = 0_usize;

    let start = std::time::Instant::now();
    args.visit_rows(|row| {
        let path = row.path.as_path();
        if !path.is_file() {
            return Ok(ControlFlow::Continue(()));
        }

        total_files += 1;
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .unwrap_or(NO_EXTENSION);
        if let Some(count) = raw_counts.get_mut(extension) {
            *count += 1;
        } else {
            raw_counts.insert(extension.to_owned(), 1);
        }
        Ok(ControlFlow::Continue(()))
    })?;

    let mut counts = BTreeMap::<String, usize>::new();
    for (extension, count) in raw_counts {
        let normalized_extension = if extension == NO_EXTENSION {
            extension
        } else if extension.bytes().any(|byte| byte.is_ascii_uppercase()) {
            extension.to_ascii_lowercase()
        } else {
            extension
        };
        *counts.entry(normalized_extension).or_default() += count;
    }

    println!("scope: {}", scope.display());
    println!("files: {total_files}");
    for (extension, count) in counts {
        if extension == NO_EXTENSION {
            println!("{extension}\t{count}");
        } else {
            println!(".{extension}\t{count}");
        }
    }

    let elapsed = start.elapsed();
    println!("elapsed: {elapsed:.2?}");
    Ok(())
}
