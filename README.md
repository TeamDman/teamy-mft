# Teamy MFT

Command-line toolkit for upcoming MFT / storage utilities.

## Installation

```bash
cargo install --path .
```

## Commands (initial)

- `teamy-mft get-sync-dir` – Prints currently configured sync directory or `<not set>`.
- `teamy-mft set-sync-dir [path]` – Sets sync directory. If `path` omitted uses current working directory.

Configuration stored per-user in platform config directory (JSON file `sync_dir.json`).

## Roadmap

- Integrate existing MFT parsing library
- Add dump/query/diff/show subcommands
- Sync workflows leveraging configured directory

