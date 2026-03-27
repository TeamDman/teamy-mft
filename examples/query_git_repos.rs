//! Find all git repositories visible in the MFT search index.
//!
// repo[impl examples.rs-files]
//! Uses `.git$` (ends-with) so it returns exact `.git` directories without
//! matching `.gitignore`, `.github`, etc.
//!
//! # Usage
//!
//! ```bash
//! cargo run --example query_git_repos
//! ```
//!
//! Requires a synced MFT index. Run `teamy-mft sync` first if needed.

use teamy_mft::cli::command::query::QueryArgs;

fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    // `.git$` means "path ends with .git" — matches dirs named exactly `.git`
    for path in QueryArgs::new(".git$").invoke()? {
        // path is the .git dir; print its parent (the repo root)
        if let Some(repo_root) = path.parent() {
            println!("{}", repo_root.display());
        }
    }

    Ok(())
}
