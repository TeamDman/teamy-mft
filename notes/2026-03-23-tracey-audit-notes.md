# Tracey Audit Notes

This file holds non-spec observations from the first tracey setup pass.

## Documentation Drift

- `README.md` lists a `check` command, but the implemented command surface in `src/cli/command/command_cli.rs` currently exposes `sync`, `list-paths`, `get-sync-dir`, `set-sync-dir`, and `query`.

## Publishing Polish Gaps

- `README.md` does not currently appear to include package-page badges.
- `README.md` does not currently appear to include a media demonstration.
- We want a GitHub Actions workflow that runs `tracey query` so the repository can expose a badge demonstrating current spec conformity.

## Spec And Coverage Notes

- The search-index artifact spec was split out from the MFT assumptions spec into its own index-file-format spec.
- Large parts of the parser, path-resolution, and query engine remain intentionally uncovered in this first pass and should be expanded iteratively.