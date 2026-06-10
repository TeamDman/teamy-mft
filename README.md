# Teamy MFT

<!-- repo[impl readme.explanation] -->
<!-- repo[impl readme.package-page-badges] -->

[![crates.io](https://img.shields.io/crates/v/teamy-mft.svg)](https://crates.io/crates/teamy-mft)

Command-line toolkit for interacting with the NTFS Master File Table on Windows.

## Installation

<!-- repo[impl readme.code-example] -->
```bash
cargo install --path .
```

## Demo

<!-- repo[impl readme.media-demo] -->

[YouTube demo](https://youtu.be/bCeMz1AZw-E)

```
# Sync the MFT for all drives (auto-elevates via UAC if needed)
teamy-mft sync

# Inspect cache freshness for the indexed query files on each drive
teamy-mft status

# Query indexed paths
teamy-mft query ".mp4> album" ".opus> album" ".mp3> album"

# Add a privacy-preserving exclusion rule to the default profile
teamy-mft rules add exclude FirstName

# Add a profile-specific include rule with explicit ordering
teamy-mft rules add --profile mc-modding --order 100 include "C:\\Repos\\Minecraft\\**\\*.java"

# Query with a profile-specific ruleset
teamy-mft query ".java>" --profile mc-modding

# See which rule files are active for one profile
teamy-mft rules list --profile mc-modding

# See which profiles exist
teamy-mft profile list
```

## Library Usage

<!-- repo[impl examples.readme-snippet] -->

```rust
use teamy_mft::cli::command::query::QueryArgs;
use teamy_mft::query::QueryExecutionOptions;
use teamy_mft::query::QueryIgnoreBehavior;

fn main() -> eyre::Result<()> {
    // By default, queries honor discovered `.teamy_mft_rules` files.
    for path in QueryArgs::new("<.git>").invoke()? {
        if let Some(repo_root) = path.parent() {
            println!("{} ({})", repo_root.display(), path.display());
        }
    }

    // Library callers can also opt out explicitly.
    let _unfiltered = QueryArgs::new("FirstName").invoke_with_options(QueryExecutionOptions {
        ignore: QueryIgnoreBehavior::Disabled,
    })?;

    Ok(())
}
```

See [`examples/query_git_repos.rs`](examples/query_git_repos.rs) for a runnable version.

## CLI

```
❯ teamy-mft --help     
teamy-mft.exe 0.5.0 (rev 12b4a4f)

Teamy MFT command-line interface.
Environment variables:
- `TEAMY_MFT_SYNC_DIR`: override the persisted sync directory for commands that read cached data.

USAGE:
    teamy-mft.exe [OPTIONS] <COMMAND>

OPTIONS:
        --debug
            Enable debug logging
        --log-filter <STRING>
        --log-file <STRING>
        --json <STRING>
            Emit structured JSON logs alongside stderr output.
        --console-pid <U32>
            Console PID for console reuse (hidden)
    -h, --help
            Show help message and exit.
    -V, --version
            Show version and exit.
        --completions <bash,zsh,fish>
            Generate shell completions.

COMMANDS:
    sync
            Write .mft and .mft_search_index files (will auto-elevate via UAC if not already running as administrator)
    status
        Show per-drive cache freshness for `.mft` and `.mft_search_index` files
    list-paths
            Produce newline-delimited list of file paths for matching drives from cached .mft files
    get-sync-dir
            Get the currently configured sync directory
    set-sync-dir
            Set the sync directory (defaults to current directory if omitted)
    query
            Query indexed file paths (substring match) across cached `.mft_search_index` files


Implementation:
    src\cli\mod.rs
    https://github.com/TeamDman/teamy-mft/blob/12b4a4f/src/cli/mod.rs
```

