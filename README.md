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

# Query indexed paths
teamy-mft query ".mp4$ album" ".opus$ album" ".mp3$ album"
```

## Commands

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
