//! Find all git repositories visible in the MFT search index.
//!
// repo[impl examples.rs-files]
//! Uses `<.git>` (exact terminal-segment match) so it returns exact `.git`
//! directories without matching `.gitignore`, `.github`, `project.git`, etc.
//!
//! # Usage
//!
//! ```bash
//! cargo run --example query_git_repos
//! ```
//!
//! Requires a synced MFT index. Run `teamy-mft sync` first if needed.

use teamy_mft::cli::command::query::QueryArgs;
use teamy_mft::query::ControlFlow;
use teamy_mft::query::QueryNeedle;
use teamy_mft::query::QueryPlan;
use teamy_mft::query::QueryRule;

fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    let query = QueryArgs {
        plan: QueryPlan::single_rule(QueryRule::EqualsCaseInsensitive(QueryNeedle::new(
            ".git",
        ))),
        ..Default::default()
    };
    query.visit_rows(|row| {
        // path is the .git dir; print its parent (the repo root)
        if let Some(repo_root) = row.parent() {
            println!("{} ({})", repo_root.display(), row.display());
        }
        Ok(ControlFlow::Continue(()))
    })?;

    Ok(())
}
