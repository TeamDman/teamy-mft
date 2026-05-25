# Daemon Introduction And Current State

This note summarizes why the Windows daemon was introduced, what architecture we are aiming for, what is already implemented, and what still appears rough or unresolved as of 2026-05-24.

## Why The Daemon Exists

The original `teamy-mft` model was snapshot-oriented:

- read raw MFT data from selected drives
- write `<drive>.mft`
- build `<drive>.mft_search_index`
- run later queries against those cached files

That model is fast for read-only querying, but it goes stale as soon as the filesystem changes after sync. On Windows, keeping query freshness up to date without requiring every query to run elevated pushes the design toward a split system:

- an elevated component owns raw MFT access and USN journal access
- an unelevated query client reads published artifacts and optionally asks the daemon for fresher results

This is the motivation for the machine-managed Windows service / daemon path.

## Intended Architecture

The current target shape is:

1. `install`
   - elevated
   - registers the Windows service
   - writes machine config under `%ProgramData%`
   - configures ACLs for the machine root, cache root, service, and named pipe
   - does not perform the expensive sync/bootstrap work by default

2. `sync`
   - normal user command
   - starts the daemon if needed
   - sends an IPC request to the daemon
   - the daemon performs privileged sync work

3. `query`
   - normal user command
   - prefers the daemon for live drives when available
   - falls back to published disk artifacts when needed

4. Published machine-managed state per drive
   - `<drive>.mft`
   - `<drive>.mft_search_index`
   - `<drive>.mft_overlay_search_index`
   - `<drive>.mft_checkpoint.json`

5. Live behavior
   - journal-capable NTFS drives use USN-backed live refresh
   - drives without an active USN journal still participate as snapshot-only drives

## Why Install And Sync Were Separated

Early iterations tried to make `install` do too much:

- create machine config
- register the service
- repair ACLs
- run an initial MFT/index/bootstrap pass

That made installation fragile, because every drive/journal/cache problem surfaced during setup. The current direction is intentionally simpler:

- `install` provisions the machine-managed environment
- `sync` asks the daemon to build or refresh published state

This keeps one-time elevation separate from the daemon’s ongoing privileged work.

## Current User-Facing Workflow

The intended local workflow is:

```powershell
.\install.ps1
teamy-mft install --force --sync-dir G:\Programming\Caches\MFT_FILES\
teamy-mft sync
teamy-mft query <terms>
teamy-mft status
```

Important details:

- `install` now refuses to register a service from `target\debug` or `target\release`
- `install --force` uninstalls the existing service before reinstalling
- service-exists checks happen before elevation when possible
- the machine-managed cache uses the exact `--sync-dir` path supplied by the user

## Current Implementation State

### Implemented

The following major pieces are already in place:

- top-level `install`, `uninstall`, and hidden/internal `daemon` commands
- machine config persisted under `%ProgramData%\teamy_mft\machine_config.json`
- Windows service registration pointing at:
  - `"<installed teamy-mft.exe>" daemon --service`
- named-pipe IPC between client commands and the daemon
- daemon runtime with:
  - idle timeout
  - per-drive live state loading
  - periodic refresh attempts for loaded live drives
  - dirty overlay flush on shutdown/idle
- machine-managed query routing
- machine-managed sync routing
- live USN-backed drive state for journal-capable NTFS drives
- snapshot-only participation for drives without an active USN journal
- denser tracing/logging around install, service startup, daemon request handling, USN interactions, and sync coordination

### Important Behavioral Shifts

The current design intentionally dropped the old “legacy sync-dir” runtime path from the main code path. The machine-managed model is now the primary direction:

- machine config decides the cache root
- the service owns privileged operations
- `TEAMY_MFT_SYNC_DIR` is no longer meant to be the core service configuration mechanism

Some older spec/README material still reflects the previous CLI shape and should be updated later.

## Journaled vs Snapshot-Only Drives

Not every Windows drive we can snapshot from can also participate in live journaling.

### Journaled drives

These are NTFS drives with an active USN journal. For these drives the daemon can:

- record a snapshot checkpoint
- replay USN changes after the published snapshot
- keep a live in-memory view fresher than the last full sync

### Snapshot-only drives

These are drives where MFT-based snapshot/index sync still works, but there is no active USN journal available. This can happen on some removable or external media.

For these drives:

- `sync` should still build `.mft` and `.mft_search_index`
- `query` should still work from published disk artifacts
- live daemon freshness is not currently available through USN replay

This split is intentional and expected.

## Debugging And Robustness Work Already Added

Several bugs were found while exercising the daemon path and were fixed:

### Service install / config / ACL fixes

- install now avoids target build outputs
- install can force-reinstall
- machine config write path repairs ownership and ACLs on stale config files
- ACL repair uses `takeown.exe` and `icacls.exe`
- stale cache root permissions are repaired more aggressively

### Daemon sync / runtime fixes

- daemon sync no longer tries to create a nested Tokio runtime inside an already-running Tokio runtime
- request handling now wraps query and sync execution in panic boundaries and returns a structured daemon error instead of silently dropping the pipe
- the named-pipe server loop no longer creates the next instance too early while `max_instances(1)` is in effect
- the named pipe now uses explicit byte-stream framing rather than relying on Windows message mode while also doing our own length-prefix framing
- daemon request handling no longer force-disconnects the pipe immediately after writing the response

### Cache permission repair improvements

- daemon-side sync now attempts cache-root repair before sync
- daemon-side sync also attempts per-file artifact repair for:
  - `.mft`
  - `.mft_search_index`
  - `.mft_overlay_search_index`
  - `.mft_checkpoint.json`
- `icacls` output parsing now distinguishes between:
  - `Failed processing 0 files`
  - real non-zero failure counts

## Current Rough Edges

The daemon path is much farther along than it was at introduction time, but it is not fully settled yet.

### 1. Machine-managed sync is still being hardened against stale artifact ACLs

During live testing, old artifacts in the chosen cache directory were able to poison overwrite attempts even when the directory itself was writable. Clearing the directory is currently the cleanest way to get back to a known-good state.

This is one of the reasons we have been iterating on the cache ACL repair path.

### 2. Product/spec docs lag behind the implementation

Some existing documentation still describes:

- `get-sync-dir`
- `set-sync-dir`
- legacy sync-dir environment override behavior

That no longer represents the intended machine-managed core. Those docs should be reconciled.

### 3. Live end-to-end validation still needs more runtime proof

We have:

- unit and integration coverage for many data-structure and parsing behaviors
- an ignored smoke test for live-refresh behavior that requires elevation and NTFS journal access

But we still need more repeated real-machine validation of:

- install
- sync
- daemon startup/shutdown
- query after live filesystem mutations

especially across mixed drive capability sets.

## Current Practical Guidance

If working on the daemon path right now:

1. Treat `install` as provisioning only.
2. Use `sync` as the real machine-managed bootstrap/repair entrypoint.
3. Expect mixed drive capabilities:
   - some live
   - some snapshot-only
4. Prefer tracing/logging-backed diagnosis over speculative changes.
5. When a sync failure mentions a specific artifact path, suspect stale file-level ACLs before assuming the entire cache root is broken.

## Near-Term Next Steps

The most useful next steps appear to be:

1. Re-run the full machine-managed flow against a clean cache root and confirm:
   - install succeeds
   - sync succeeds
   - status reports the expected machine-managed state

2. Exercise the intended live behavior:
   - query for a missing path
   - create or rename a file on a journal-capable drive
   - query again
   - verify the daemon-backed result changes without a full resync

3. Reconcile product docs and CLI docs with the machine-managed model.

4. Add more explicit daemon logging around:
   - cache artifact overwrite decisions
   - per-drive mode selection
   - published checkpoint updates
   - fallback from live to disk-backed query paths

## Bottom Line

The daemon introduction is no longer just a sketch. The service, IPC, live-drive state, and machine-managed sync/query routing are real and partially working. The major remaining issues are operational hardening issues, especially around Windows cache/file permissions and repeated real-machine validation, not a lack of overall architecture.
