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
use teamy_mft::query::QueryPlan;
use teamy_mft::query::QueryRule;

fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    let scope = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    let scope = dunce::canonicalize(&scope)?;

    let mut plan = QueryPlan::single_rule(QueryRule::MatchAll);
    plan.r#in = Some(scope.to_string_lossy().into_owned());
    let args = QueryArgs {
        plan,
        ..Default::default()
    };

    let mut counts = BTreeMap::<String, usize>::new();
    let mut total_files = 0_usize;

    for row in args.collect_rows()? {
        let path = row.path.as_path();
        if !path.is_file() {
            continue;
        }

        total_files += 1;
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .map_or_else(|| String::from("(no extension)"), |value| {
                format!(".{}", value.to_ascii_lowercase())
            });
        *counts.entry(extension).or_default() += 1;
    }

    println!("scope: {}", scope.display());
    println!("files: {total_files}");
    for (extension, count) in counts {
        println!("{extension}\t{count}");
    }

    Ok(())
}