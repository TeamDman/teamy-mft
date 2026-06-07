# CLI

This specification covers the current user-facing command line behavior exposed by `teamy-mft`.

## Command Surface

cli[command.surface.core]
The CLI must expose the `sync`, `install`, `uninstall`, `list-paths`, `rules`, `profile`, `status`, `query`, and `tray` commands.

cli[help.describes-machine-install]
The top-level CLI help output must mention the `install` command so machine-managed setup is discoverable.

## Parser Model

cli[parser.args-consistent]
The structured CLI model must serialize to command line arguments consistently for parse-safe values.

cli[parser.roundtrip]
The structured CLI model must roundtrip through argument serialization and parsing for parse-safe values.

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
