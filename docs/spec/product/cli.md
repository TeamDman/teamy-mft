# CLI

This specification covers the current user-facing command line behavior exposed by `teamy-mft`.

## Command Surface

cli[command.surface.core]
The CLI must expose the `sync`, `list-paths`, `get-sync-dir`, `set-sync-dir`, and `query` commands.

## Parser Model

cli[parser.args-consistent]
The structured CLI model must serialize to command line arguments consistently for parse-safe values.

cli[parser.roundtrip]
The structured CLI model must roundtrip through argument serialization and parsing for parse-safe values.

## Sync Directory

cli[sync-dir.env-overrides-persisted]
If `TEAMY_MFT_SYNC_DIR` is set to a non-empty value, it must take precedence over the persisted sync directory file.

cli[sync-dir.persisted-read]
If no overriding environment variable is present, the CLI must read the persisted sync directory from the configured persistence file.

cli[sync-dir.persisted-write]
The CLI must persist the configured sync directory as a UTF-8 text file for later runs.

## Cached MFT Traversal

cli[command.list-paths.cached-mft-input]
The `list-paths` command must traverse cached `.mft` files for the selected drive letters.

## Querying

cli[command.query.drive-pattern-selection]
The `query` command must restrict its work to cached search indexes whose drive letters match the selected drive pattern.

cli[command.query.scope-filter]
The `query` command must support restricting results to an exact path or a directory subtree.

cli[command.query.deleted-filter]
The `query` command must support including deleted paths or limiting output to deleted paths only.