# Project Summary

This workspace currently contains multiple related Rust crates and supporting files focused on parsing and analyzing the Windows NTFS Master File Table (MFT) and providing higher-level storage usage tooling.

## Top-Level Folders

- `teamy-mft/` (root placeholder repo folder; minimal content)
- `mft/` (core library + example binary for parsing NTFS MFT structures)
- `storage-usage-v2/` (application crate leveraging the `mft` crate to dump, query, diff, and display MFT information with CLI and TUI features)

---
## Repository Root: `teamy-mft/`
### Files
- `LICENSE` – Root project license placeholder (may duplicate or supersede crate-specific licenses).
- `README.md` – Minimal readme (currently just a header).

---
## Crate: `mft/`
Core Rust library implementing safe parsing of NTFS Master File Table records and attributes. Provides data structures, iterators, attribute decoding, and a convenience binary `mft_dump` when feature `mft_dump` is enabled.

### Top-Level Files
- `Cargo.toml` – Crate metadata, dependencies, features (`mft_dump`), benches, and binary target.
- `Cargo.lock` – Locked dependency versions.
- `build.rs` – Build script (not inspected yet; likely for skeptic/doc tests or codegen).
- `CHANGELOG.md` – Version history.
- `LICENSE-APACHE`, `LICENSE-MIT` – Dual licensing info.
- `README.md` – Library overview, usage examples, feature list, install instructions.
- `release.py` – Release automation script (probably tagging / publishing; not opened).

### Directories
- `samples/` – Sample MFT-related binary data and specific entry examples (used for testing / demonstration). Contains files like `MFT`, `entry_single_file`, etc.
- `src/` – Library source code.
- `target/` – Build artifacts (ignored for rewrite purposes).
- `testdata/` – Additional test fixture data sets (attribute variations, long names, etc.).
- `tests/` – Integration tests (contents not yet read; likely exercising CLI or parsing against `testdata`).

### `src/` Contents
- `lib.rs` – Crate root, exports public modules (`attribute`, `csv`, `entry`, `err`, `mft`), re-exports key types (`MftParser`, `MftEntry`, `EntryHeader`, attribute structs), sets lint directives.
- `mft.rs` – Implements `MftParser<T: Read + Seek>`: construction from file/buffer, entry iteration, path reconstruction with LRU caching, and entry count logic.
- `entry.rs` – Defines `MftEntry` and `EntryHeader`; parsing logic for entries, fixup array application, attribute iteration (with filtering), name resolution, and serialization integration. Also defines `EntryFlags`.
- `csv.rs` – (Not opened) Presumably CSV serialization / export utilities for entries/attributes.
- `err.rs` – (Not opened) Error enum / `Result` type encapsulating parsing failures (e.g., signatures, attribute decoding, I/O bounds, data runs decode errors).
- `macros.rs` – (Not opened) Likely helper macros (e.g., serialization for bitflags, error generation).
- `utils.rs` – (Not opened) Likely shared helpers (UTF-16 string decoding, alignment, reading helpers).
- `attribute/` – Submodule tree for each NTFS attribute type plus generic handling.
  - `mod.rs` – Central attribute definitions: `MftAttribute`, `MftAttributeContent` enum (typed variants: X10, X20, X30, etc.), conversion helpers (`into_*`), attribute type enum `MftAttributeType`, and bitflag structs (`FileAttributeFlags`, `AttributeDataFlags`). Delegates to resident/non-resident loaders.
  - `header.rs` – Parsing of attribute record headers (resident & non-resident variants), capturing lengths, offsets, flags, and names; provides `MftAttributeHeader`, `ResidentHeader`, `NonResidentHeader`.
  - `raw.rs` – (Not opened) Likely container for unparsed/unknown resident attribute data.
  - `x10.rs` – (Not opened) Parser/model for Standard Information attribute.
  - `x20.rs` – (Not opened) Parser/model for Attribute List attribute (multi-record aggregation).
  - `x30.rs` – (Opened partially via re-exports) File Name attribute parsing & namespace classification.
  - `x40.rs` – (Not opened) Object ID attribute parsing.
  - `x80.rs` – (Not opened) Data attribute parsing (resident data case) – non-resident handled elsewhere.
  - `x90.rs` – (Not opened) Index Root attribute parsing.
  - `non_resident_attr.rs` – Wrapper for non-resident data runs (`NonResidentAttr`) decoding mapping pairs.
  - `data_run.rs` – Implements decoding of NTFS data runs (supporting sparse runs) into `DataRun` list.
  
- `bin/mft_dump.rs` – (Not opened) CLI binary implementation leveraging the library (enabled with feature `mft_dump`).
- `benches/benchmark.rs` – Criterion benchmark harness (performance measurement of parser operations).
- `tests/` (module-internal) – `fixtures.rs`, test helpers for loading sample MFT; additional parser tests.

### Key Concepts Encapsulated in `mft`
- Safe, zero-copy-ish parsing via buffered reads and cursor iteration.
- Attribute abstraction unifying resident/non-resident forms.
- Data run decoding for non-resident DATA attributes (cluster mapping).
- Path reconstruction with caching for parent traversal.
- Serialization (Serde) for converting entries & attributes to JSON/CSV.

---
## Crate: `storage-usage-v2/`
High-level application providing multi-command CLI and potential TUI for interacting with NTFS volumes and offline MFT dumps. Uses the `mft` crate internally (patched to local path). Focus areas: dumping raw MFT from live volume, querying contents, diffing two dumps, showing stats, and possibly interactive visualization.

### Top-Level Files
- `Cargo.toml` – Package metadata; depends on `mft` crate (patched locally). Includes Windows-specific and CLI/UI crates (`windows`, `ratatui`, `tracing`, etc.).
- `Cargo.lock` – Dependency lockfile.
- `README.md` – Detailed user-facing documentation for commands and workflow (dump, query, show, diff, elevation, technical implementation specifics).
- `rustfmt.toml` – Formatting configuration (not opened).
- `TODO.md` – (Not opened) Probably future enhancements list.
- `update.ps1` – (Not opened) PowerShell helper script (maybe updating dependencies / running tasks).
- `target/` – Build artifacts (ignore for rewrite design).

### `src/` Contents
- `lib.rs` – Module declarations re-exporting internal submodules.
- `main.rs` – CLI entrypoint: installs color-eyre, parses arguments (clap), initializes tracing, optionally reuses console, dispatches `cli.run()`.
- `config.rs` – (Not opened) Likely handles persistent config (directories-next usage) or runtime settings.
- `console_reuse.rs` – (Not opened) Utilities to keep/reuse Windows console window on elevation / relaunch.
- `init_tracing.rs` – (Not opened) Initializes `tracing` subscriber based on log level/global args.
- `mft_dump.rs` – Core logic to dump live volume MFT using proper data run parsing, privilege elevation, filesystem validation, boot sector & record 0 interpretation, data run following, chunked reads, and output writing.
- `mft_query.rs` – (Not opened) Likely loads MFT file and filters entries by pattern(s), supports glob/wildcard & case-insensitive matching, limit & full path toggles.
- `mft_show.rs` – (Not opened) Likely statistical summarizer: counts, type distributions, sample paths, performance metrics.
- `mft_diff.rs` – (Not opened) Compares two MFT dumps (possibly entry counts, sizes, byte-level diffs with configurable limits).
- `to_args.rs` – (Not opened) Probably helper trait(s) converting internal config/state to CLI argument structures or pattern objects.
- `win_elevation.rs` – (Not opened) Functions to detect elevation, relaunch with admin rights, and manipulate privileges (paired with privilege enabling in `mft_dump.rs`).
- `win_handles.rs` – (Not opened) Helpers to obtain Win32 HANDLEs for volumes / devices safely (wrapping unsafe Windows API calls).
- `win_strings.rs` – (Not opened) Utilities for constructing / converting wide strings (UTF-16) for Windows API calls.
- `tui/` – Text-based UI components (ratatui-based) for interactive progress or visualization.
  - `mod.rs` – (Not opened) Root of TUI module; exports widget submodules & app logic.
  - `app.rs` – (Not opened) Primary application state & event loop model for TUI.
  - `mainbound_message.rs` – (Not opened) Likely cross-thread messaging types between worker & UI.
  - `progress.rs` – (Not opened) Progress indicators / spinners / bars.
  - `worker.rs` – (Not opened) Background thread performing long operations while UI updates.
  - `widgets/` – Collection of custom TUI widgets and tabs.
    - `mod.rs` – (Not opened) Aggregates widget modules.
    - `tabs/` – Tabbed interface components.
      - `app_tab.rs` – (Not opened) Probably defines an enum/state for each tab.
      - `app_tabs.rs` – (Not opened) Logic to render tab bar and switch views.
      - `errors_tab.rs` – (Not opened) Displays captured errors/logs.
      - `keyboard_response.rs` – (Not opened) Key binding help overlay or dynamic response mapping.
      - `overview_tab.rs` – (Not opened) High-level stats & summaries.
      - `search_tab.rs` – (Not opened) Interactive search within loaded MFT.
      - `visualizer_tab.rs` – (Not opened) Possibly a graphical cluster/fragmentation visualization.

### `src/cli/` (Command-line argument and command handling)
- `mod.rs` – (Not opened) Likely defines the top-level `Cli` struct with subcommand enums and dispatch logic (`run()` invoked in `main.rs`).
- `global_args.rs` – (Not opened) Shared global flags (log level, debug, console reuse).
- `action.rs` – (Not opened) Trait(s) or abstractions for executable actions / subcommands.
- `config_action.rs` – (Not opened) Handles config-related commands (view/edit config). 
- `drive_letter_pattern.rs` – (Not opened) Validation/parsing for drive letter arguments.
- `elevation_action.rs` – (Not opened) CLI commands to elevate or test privileges.
- `elevation_check_action.rs` – (Not opened) Implements `elevation check` subcommand.
- `elevation_test_action.rs` – (Not opened) Implements `elevation test` subcommand / diagnostics.
- `mft_action.rs` – (Not opened) Parent subcommand grouping for MFT operations.
- `mft_dump_action.rs` – (Not opened) Parses args for `mft dump` and invokes `dump_mft_to_file` logic.
- `mft_query_action.rs` – (Not opened) Parses args for `mft query` and invokes search logic.
- `mft_show_action.rs` – (Not opened) Parses args for `mft show` and invokes statistics display.
- `mft_diff_action.rs` – (Not opened) Parses args for `mft diff` and invokes diff engine.
- `mft_sync_action.rs` – (Not opened) Potential future command (maybe synchronizing state or incremental differences).

### Key Concepts Encapsulated in `storage-usage-v2`
- Windows privilege management & elevation workflows.
- Direct NTFS on-disk structure parsing for robust MFT extraction (no reliance on simple linear reads).
- Modular CLI with subcommand-per-operation architecture.
- Potential interactive terminal UI for visualization / progress / searching.
- Integration with `mft` crate for parsing, iteration, query, diff operations.

---
## Suggested Migration / Rewrite Considerations
(For planning; not part of original request but useful context.)
- Separate pure parsing (library) from OS-specific extraction logic (volume reading & privilege escalation).
- Provide trait-based abstraction for MFT data sources (in-memory, file, live volume).
- Consolidate data run decoding logic (exists in both crates: core `mft` and custom implementation in `storage-usage-v2::mft_dump`).
- Improve error hierarchy consistency across crates.
- Evaluate async vs sync (likely keep sync due to disk/WinAPI constraints unless TUI benefits from async).
- Strengthen test coverage for edge cases: sparse data runs, corrupt headers, self-referential entries.

---
## Legend for Attribute Modules (mft crate)
- X10: Standard Information (timestamps, flags, link count context)
- X20: Attribute List (multi-record attribute reference listing)
- X30: File Name (parent reference, namespaces, name + metadata)
- X40: Object ID
- X80: Data (file content or resident payload)
- X90: Index Root (directory indexing base)
- NonResidentAttr/DataRun: Non-resident data mapping (cluster runs, including sparse handling)

---
## Gaps / Files Not Opened (May Need Inspection During Rewrite)
List of files referenced but not yet summarized in detail (open if deeper analysis required):
- `mft/src/csv.rs`
- `mft/src/err.rs`
- `mft/src/macros.rs`
- `mft/src/utils.rs`
- `mft/src/attribute/raw.rs`
- `mft/src/attribute/x10.rs`
- `mft/src/attribute/x20.rs`
- `mft/src/attribute/x30.rs` (partially inferred)
- `mft/src/attribute/x40.rs`
- `mft/src/attribute/x80.rs`
- `mft/src/attribute/x90.rs`
- `mft/src/bin/mft_dump.rs`
- `mft/src/benches/benchmark.rs`
- All `tests/` & `testdata/` contents beyond what was needed.
- All `storage-usage-v2/src/cli/*.rs` implementation details.
- `storage-usage-v2/src/*` modules not opened (query/show/diff, etc.).
- `storage-usage-v2/src/tui/**/*` internals.
- Support scripts: `release.py`, `update.ps1`, `TODO.md`, etc.

These can be expanded upon request.

---
Generated for rewrite planning: provides a quick map from functionality to source files so selective porting/refactoring can proceed efficiently.
