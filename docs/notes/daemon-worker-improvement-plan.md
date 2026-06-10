# Daemon Worker Improvement Plan

## Goal

Refactor the daemon and per-drive worker architecture so the same worker model
can support both of these modes cleanly:

- `LiveObserved`: the current daemon-service behavior that loads live drive
  state, observes USN journal changes, refreshes in memory, and flushes back to
  published artifacts when appropriate.
- `PublishedIndexOnly`: an in-process or non-service behavior for repeated
  queries that should reuse cached index state, preserve cancellation safety,
  and avoid live observation work entirely.

The end state should let Cloud Terrastodon consciously opt into an in-process
read-only daemon-style query cache instead of accidentally getting repeated
`no-daemon` queries via one-off `QueryArgs` calls.

## Current Status

- Done so far:
  - Reviewed the current daemon, worker, published-index query, and live-drive
    query paths in `src/machine/daemon.rs`, `src/machine/live_drive_state.rs`,
    `src/query/search_index_query.rs`, and the `cloud-terrastodon` software
    query call site.
  - Confirmed that the drive worker loop already provides single-threaded
    per-drive mutation/query coordination.
  - Confirmed that live daemon queries currently row-crawl projected paths
    instead of using the indexed query engine, while published-index queries use
    the fast search-index matcher.
  - Confirmed that daemon warmup is gradual and policy-driven in code, but not
    yet configurable by runtime mode.
  - Added the dedicated daemon-worker spec at
    `docs/spec/product/daemon-worker-runtime.md`.
  - Registered the new spec in `.config/tracey/config.styx`.
  - Introduced an explicit `DriveWorkerRuntimeMode` in
    `src/machine/daemon.rs`, while keeping service-created workers on
    `LiveObserved`.
  - Added a `PublishedIndexOnly` query branch in `DriveWorkerState` that can
    answer from published indexes without requiring live state.
  - Added a regression test proving `PublishedIndexOnly` can query a published
    base index without loading live drive state.
  - Refactored `src/cli/command/query/query_cli.rs` so `QueryArgs::collect_rows()`
    now prepares a `QueryRowStream` and runs one shared row-collection step for
    both daemon and non-daemon execution.
  - Moved daemon-specific RPC launch, log draining, and Ctrl+C forwarding behind
    internal cleanup helpers instead of keeping a separate top-level collection
    flow inside `collect_rows()`.
  - Added a regression test proving the daemon Ctrl+C forwarder can shut down
    cleanly without hanging when no interrupt arrives.
  - Extracted the shared query backend seam into
    `src/query/query_runtime.rs` so backend selection and streamed-query cleanup
    now live in `src/query` instead of inside the CLI entry point.
  - Simplified `QueryArgs::collect_rows()` again so it now validates arguments,
    selects a `QueryRuntime`, and delegates row collection rather than owning
    daemon RPC orchestration details directly.
  - Added `src/query/query_session.rs` with the first reusable `QuerySession`
    primitive.
  - `QuerySession::published_index_only()` now caches already-open published
    search index mappings per drive across repeated queries instead of
    re-opening those index files every time.
  - Refactored `src/query/search_index_query.rs` so query execution can run
    against an already-open `MappedSearchIndex`, which the new session reuses.
  - Updated `cloud-terrastodon/crates/software/src/lib.rs` so software
    detection now creates one `QuerySession::published_index_only()` and reuses
    it across its repeated queries.
  - Reworked `LiveDriveState::query_with_cancel()` so live-observed daemon
    queries now execute against the current in-memory search-index cache rather
    than row-crawling projected paths as the primary matching path.
  - Added cancel-aware live query cache rebuild/projection helpers so indexed
    live queries still stop cleanly when cancellation is requested during cache
    preparation.
  - Added a live-query regression test proving the indexed path still finds the
    expected row and populates the in-memory query cache.
  - Added a live-query cancellation regression test proving an already-cancelled
    request returns no rows and does not populate the current query cache.
  - Added focused live-query parity coverage for:
    - limit handling after deleted-row filtering
    - `only_deleted` semantics on indexed live rows
    - `.teamy_mft_rules`-driven filtered row classification through the live
      indexed path
    - degraded error reporting when the in-memory live query cache is corrupt
    - directory and exact-file scope filtering against real canonicalized temp
      paths rather than purely synthetic `C:\...` fixtures
    - invalid query scope paths surfacing as `RequestInvalid`
    - malformed discovered `.teamy_mft_rules` files surfacing as `Degraded`
  - Improved `LiveDriveState` error mapping so the indexed live-query path now
    preserves the chained `eyre` error text when converting failures into
    `MachineError` responses.
  - Extracted a shared parsed-search-index row visitor in
    `src/query/search_index_query.rs` and switched `LiveDriveState` to use it,
    so live-backed and published-index query paths now share the same
    parsed-index matching/materialization logic instead of duplicating it.
  - Added `QuerySession::collect_rows_with_cancel(...)` so the in-process
    published-index session can now stop early on a best-effort cancellation
    signal while still reusing cached drive state.
  - Switched `QuerySession` to reuse the shared parsed-search-index visitor for
    its cached published-index path, instead of only using the higher-level
    one-shot mapped-index query helper.
  - Added `cloud-terrastodon` `list_software_counts_with_cancel(...)` so the
    software-list helper can use the new session cancellation surface without
    losing the explicit repeated-query cache choice.
  - Switched `QueryRuntime::PublishedIndexOnly` to prepare its local
    `QueryRowStream` through `QuerySession::published_index_only()` instead of
    the older one-off `DiskQueryExecutor` path, so the shared `--no-daemon`
    runtime surface now leans on the same repeated-query cache abstraction that
    Cloud Terrastodon uses.
  - Added a dedicated local Ctrl+C flag forwarder for the session-backed query
    runtime path so local streamed queries still stop cleanly and their helper
    threads shut down explicitly.
  - Added `QuerySession::visit_rows_with_cancel(...)` and
    `QuerySession::count_rows_with_cancel(...)` so the session now has a
    visitor/count surface in addition to bulk collection.
  - Switched the session-backed local runtime path to stream rows from the
    session visitor directly instead of collecting a full intermediate `Vec`
    before publishing to `QueryRowStream`.
  - Updated Cloud Terrastodon software detection to use the session count API
    rather than collecting rows only to call `.len()`.
  - Wired the `cloud-terrastodon` `software list` command itself to the
    cancel-aware software helper using a local `tokio::signal::ctrl_c()` task,
    so the top-level command path now preserves graceful cancellation instead of
    only exposing it at the library layer.
  - Added worker-layer daemon regressions for runtime-mode and warmup-policy
    behavior:
    - `PublishedIndexOnly` workers ignore `Warm` requests instead of trying to
      load live state
    - `warm_next_drive_worker(...)` creates a `LiveObserved` worker that can
      degrade into snapshot-only mode when the published cache exists but live
      load fails
  - Moved local published-index stream production behind
    `QuerySession::spawn_stream(...)`, so `QueryRuntime::PublishedIndexOnly`
    now delegates to the session facade instead of owning session-specific
    stream setup details itself.
  - Documented the intended steady-state boundary in code:
    - `QueryRuntime` is the one-shot backend selector for CLI/sessionless
      callers
    - `QuerySession` is the persistent in-process repeated-query facade
    - `DiskQueryExecutor` remains as the specialized direct local streaming
      helper that still exposes explicit `QueryFilterBehavior` control
  - Added CLI-level query runtime tests that lock down the current user-facing
    flag semantics:
    - default query args and `--no-daemon` both select the local
      `PublishedIndexOnly` runtime
    - `--daemon` selects the daemon RPC runtime
    - conflicting `--daemon` plus `--no-daemon` now fail before profile/config
      access instead of after extra environment work
  - Added a `DiskQueryExecutor` regression proving why that helper still exists:
    explicit `QueryFilterBehavior` control can bypass auto-discovered
    `.teamy_mft_rules`, which the current `QueryRuntime`/`QuerySession`
    surfaces do not expose directly.
  - Captured the current validation baseline for this slice:
    - `cargo fmt --all`: passing in both `teamy-mft` and `cloud-terrastodon`
    - `cargo check`: passing
    - `cargo build --quiet`: passing
    - `cargo test daemon::tests --quiet`: passing
    - `cargo test published_index_only_mode_queries_base_index_without_live_state --quiet`: passing
    - `cargo test ctrl_c_sender_forwarder_finishes_when_stopped_without_interrupt --quiet`: passing
    - `cargo test ctrl_c_flag_forwarder_finishes_when_stopped_without_interrupt --quiet`: passing
    - `cargo test query_runtime --quiet`: passing
    - `cargo test query_session --quiet`: passing
    - `cargo test query_cli --quiet`: passing
    - `cargo test disk_query_executor --quiet`: passing
    - `cargo test published_session_spawn_stream_emits_matching_rows --quiet`: passing
    - `cargo test published_index_only_worker_ignores_warm_requests --quiet`: passing
    - `cargo test warm_next_drive_worker_creates_live_observed_worker_with_snapshot_fallback --quiet`: passing
    - `cargo test live_query_ --quiet`: passing
    - `cargo test live_query_returns_request_invalid_for_unresolvable_scope --quiet`: passing
    - `cargo test live_query_returns_degraded_for_invalid_discovered_rule_file --quiet`: passing
    - `cargo test live_query_preserves_directory_scope_filtering_against_canonical_paths --quiet`: passing
    - `cargo test live_query_preserves_exact_file_scope_filtering_against_canonical_paths --quiet`: passing
    - `cargo test live_query_filters_projected_paths_by_query_text --quiet`: passing
    - `cargo test live_query_cancelled_before_index_query_returns_no_rows --quiet`: passing
    - `cargo test -p cloud_terrastodon_software --quiet`: passing
    - `cargo test -p cloud_terrastodon_entrypoint --quiet`: passing
    - `cargo check` in `cloud-terrastodon`: passing
    - `tracey query validate --deny warnings`: passing
    - `tracey query unmapped --path src/machine/daemon.rs`: passes, reporting no unmapped code units
    - `tracey query uncovered`: `teamy-mft-publishing-standards/repo: 0 uncovered out of 10 rules`
    - `cargo clippy --all-targets --all-features -- -D warnings`: still failing due pre-existing repo-wide warnings outside this slice; the temporary `query_runtime.rs` `clippy::unnecessary_wraps` regression, the temporary daemon-test `clippy::unchecked_time_subtraction` regression, the current session/runtime boundary cleanup, and the new `DiskQueryExecutor` regression all validate without introducing new repo-local clippy failures
    - `check-all.ps1`: failing at the same pre-existing clippy stage before build/test/tracey execution
- Current focus:
  - Completion audit: the daemon-worker refactor described by this plan now
    satisfies its phase definitions of done from the current evidence. The only
    remaining ideas are deferred follow-ups, not required work for this scoped
    refactor.
- Deferred follow-ups:
  - Expose broader worker runtime-mode selection only if a future caller needs
    another public in-process daemon/worker construction surface beyond the
    current `QueryRuntime`/`QuerySession` APIs.
  - Improve `QuerySession` from “cached mapped index files” to a more retained
    parsed/indexed cache only if profiling shows parse-time allocations are a
    meaningful cost.
  - Revisit `DiskQueryExecutor` only if downstream callers stop needing its
    explicit `QueryFilterBehavior` control or if the runtime/session surfaces
    grow an equivalent knob.
  - Address the unrelated repo-wide clippy debt separately from this refactor
    if a fully green `cargo clippy --all-targets --all-features -D warnings`
    gate becomes a project priority.
- Next step:
  - If more work resumes here later, treat the deferred follow-ups above as
    separate optimization/cleanup tasks rather than unfinished core refactor
    work.

## Constraints And Assumptions

- Raw disk plus the USN journal remain the source of truth.
- Published search-index files remain the query-optimized serialized view, one
  drive at a time.
- Query and mutation work for a single drive should remain serialized inside one
  worker loop so we do not mutate worker state while answering a query.
- Service behavior and in-process behavior should share as much worker/runtime
  code as possible, but they do not need to share the same startup policy.
- Cloud Terrastodon wants a persistent repeated-query cache, not journal
  observation, overlay flushes, or service-style warmup by default.
- Ctrl+C and explicit cancellation must remain graceful for both RPC and
  in-process query execution.
- The `query` CLI should preserve the user-facing meaning of `--daemon` and
  `--no-daemon`, but the internal implementation is allowed to converge on one
  shared streaming/cancellation surface instead of maintaining two unrelated
  top-level query flows.
- Each phase should leave the repository in a buildable, testable state. Avoid
  “temporary” broken intermediate states when a compatibility shim, adapter, or
  deprecation path can preserve forward progress without destabilizing the tool.
- This plan intentionally does not optimize arbitrary suffix matching yet. That
  work should wait until we have real profiling evidence that it is needed.

## Product Requirements

- The daemon architecture must clearly model the roles of:
  - daemon coordinator
  - per-drive worker
  - live-observed drive state
  - published-index-only drive query state
- The per-drive worker must support disabling the “observe and update index”
  portion of its loop when running in read-only cache mode.
- Published-index-only repeated queries must avoid repeated full setup costs
  like opening and reparsing the same indexes on every call.
- Live queries should use the indexed query engine whenever possible so the
  daemon benefits from the same search-index structures as disk-backed queries.
- Cloud Terrastodon must be able to explicitly choose the in-process repeated
  query cache mode instead of implicitly relying on current `no-daemon`
  behavior.
- Cancellation behavior must remain explicit, testable, and reusable across
  service RPC and in-process callers.
- `QueryArgs::collect_rows()` should become simpler by routing both `--daemon`
  and `--no-daemon` through a common row-streaming model, with runtime mode and
  transport/backend selection deciding how rows are produced.
- Existing CLI and library call sites should continue to build across the
  refactor unless the project deliberately decides to make a breaking change and
  records that decision in this plan first.
- The work must stay resumable from this plan file alone.

## Architectural Direction

- Keep the daemon as the coordinator responsible for startup policy, worker
  registration, sync operations, status, and RPC integration.
- Keep the worker as the per-drive serialization point for query, refresh, and
  flush decisions.
- Introduce an explicit per-drive runtime mode, likely something close to:
  - `LiveObserved`
  - `PublishedIndexOnly`
- Refactor query execution behind a drive-query backend boundary so the worker
  can invoke the appropriate backend without duplicating orchestration logic.
- Treat row streaming and cancellation as the common top-level query surface.
  `--daemon` should mean “use the RPC-connected daemon runtime,” while
  `--no-daemon` should mean “use an in-process runtime,” not “use a completely
  separate collection pipeline.”
- Rework `LiveDriveState` query execution so it can answer queries using the
  in-memory search-index cache it already knows how to rebuild, rather than
  row-crawling projected paths.
- Expose a small in-process facade for repeated queries after the worker/runtime
  split is in place. The facade can be daemon-like without forcing Cloud
  Terrastodon to depend on RPC transport or service-oriented warmup policy.

## Tracey Specification Strategy

- Create a dedicated spec for daemon-worker runtime behavior.
  - Rationale: this is a distinct behavior area with its own lifecycle,
    cancellation, refresh policy, and mode-specific semantics. It is not a
    narrow extension of the existing CLI or index-file-format specs.
- Expected spec location:
  - `docs/spec/product/daemon-worker-runtime.md`
- Expected implementation surface to map:
  - `src/machine/daemon.rs`
  - `src/machine/live_drive_state.rs`
  - any new worker/runtime-mode modules created during the refactor
- Extend existing CLI spec only if new user-facing flags or commands are added
  to choose worker mode explicitly.

### Tracey Baseline Commands

Use these commands during each phase:

```powershell
tracey query status
tracey query uncovered
tracey query unmapped
tracey query unmapped --path src/machine/daemon.rs
tracey query validate --deny warnings
```

Run this after implementation coverage is under control:

```powershell
tracey query untested
```

### Current Tracey Baseline

- `tracey query validate --deny warnings`: passing
- `tracey query uncovered`: only reports repo-level publishing standards, with
  no uncovered rules there
- `tracey query unmapped --path src/machine/daemon.rs`: currently reports no
  mapped code units, which is a signal that daemon/worker behavior needs its
  own spec surface

## Phase Safety Rules

- No phase should intentionally leave `teamy-mft` unable to build.
- No phase should intentionally leave the `query` command unusable.
- Prefer additive migrations first:
  - add new runtime types behind existing call sites
  - introduce adapters/shims before removing old paths
  - switch callers after the new path is validated
  - remove obsolete code only after the replacement is proven
- If a temporary dual-path period exists, document:
  - which path is canonical
  - which path is transitional
  - what evidence is required before deleting the transitional path
- If a breaking public API or CLI behavior change becomes necessary, record it
  in `Open Decisions` and `Current Status` before landing it.

## Validation Expectations Per Phase

At the end of each phase, run and record the relevant validation commands before
marking the phase complete. Use the lightest set that still proves the phase is
safe.

Minimum validation expectations:

- `cargo build`
- targeted `cargo test` for touched query/daemon modules
- `tracey query validate --deny warnings`

Recommended when the query surface changes materially:

- `cargo test`
- a manual smoke test of:
  - `teamy-mft query <pattern>`
  - `teamy-mft query <pattern> --no-daemon`
  - `teamy-mft query <pattern> --daemon` when the local daemon environment is
    available

Record the exact commands used under `Current Status` when a phase completes.

## Phased Task Breakdown

### Phase 1: Specify The Target Runtime Model

Status:
- Complete

Objective:
- Define the intended daemon/worker behavior clearly enough that subsequent
  refactors can be checked against explicit requirements.

Tasks:
- Create the dedicated daemon-worker Tracey spec.
- Document worker responsibilities, runtime modes, cancellation guarantees,
  refresh policy, warmup policy, and published-cache fallback behavior.
- Record which behaviors are service-only, which are shared, and which are
  Cloud-Terrastodon-facing.
- Map the relevant implementation units and validate the spec set.
- Add any missing plan notes needed to preserve a safe migration path for the
  CLI query surface.

Definition of done:
- A dedicated daemon-worker spec exists and validates cleanly.
- The spec distinguishes `LiveObserved` behavior from
  `PublishedIndexOnly` behavior.
- This plan file is updated with the spec path, mapping status, and any design
  decisions made while writing the spec.
- The plan explicitly states how later phases will avoid leaving the tool in a
  broken transitional state.

Completion note:
- Added `docs/spec/product/daemon-worker-runtime.md` and registered it in
  `.config/tracey/config.styx`.
- Added initial `dwrk[...]` implementation mappings in `src/machine/daemon.rs`.
- Validated with `tracey query validate --deny warnings`.

### Phase 2: Split Worker Orchestration From Drive Runtime Mode

Status:
- Complete

Objective:
- Make worker orchestration reusable by separating “what loop we run” from
  “what backend/mode this drive uses.”

Tasks:
- Introduce an explicit runtime mode type for per-drive workers.
- Refactor worker startup and warmup decisions so the daemon can choose mode per
  drive instead of assuming live observation.
- Move mode-specific refresh/flush/query preparation behind a backend boundary
  or equivalent internal abstraction.
- Decide what common row-stream/cancellation abstraction the top-level query
  surface will use so CLI and library callers do not need separate orchestration
  code paths for daemon and non-daemon execution.
- Ensure existing daemon-service behavior remains functionally unchanged in
  `LiveObserved` mode.
- Keep existing `QueryArgs` entry points building by adding internal adapters
  instead of deleting old plumbing before the replacement path is wired through.

Definition of done:
- Per-drive workers can be constructed with an explicit mode.
- Warmup and timeout refresh behavior can be disabled for
  `PublishedIndexOnly` mode without special-case hacks in Cloud Terrastodon.
- Existing service behavior still works in `LiveObserved` mode.
- The refactor direction for `QueryArgs::collect_rows()` is explicit: one
  top-level streaming/cancellation surface, different runtime modes/backends.
- `teamy-mft` still builds and the existing `query` CLI entry points still run,
  even if some old internals remain temporarily in place behind adapters.
- Tests cover at least one service-style mode path and one read-only mode path.

Progress note:
- `DriveWorkerRuntimeMode` exists and is threaded into `DriveWorkerState`.
- Service-created workers still use `LiveObserved`.
- `PublishedIndexOnly` now has a working published-index query branch covered by
  a regression test.
- `QueryArgs::collect_rows()` now routes through `QueryRuntime`, keeping the CLI
  focused on validation and user-facing limits while `src/query` owns backend
  preparation and cleanup.
- The daemon path now cleans up its Ctrl+C forwarder explicitly instead of
  leaving that helper thread detached for the lifetime of the process.
- A first `QuerySession` now exists and is already used by Cloud Terrastodon to
  reuse cached published index mappings across repeated software detection
  queries.
- The long-term relationship between `QueryRuntime` and `QuerySession` is now
  explicit in code: `QueryRuntime` is the one-shot backend selector, while
  `QuerySession` is the persistent in-process repeated-query facade.
- This phase now satisfies its definition of done from the current evidence.

Completion note:
- Per-drive workers are now mode-aware through `DriveWorkerRuntimeMode`, and
  the read-only `PublishedIndexOnly` path has targeted regression coverage.
- The shared top-level query surface is now explicit through `QueryRuntime`,
  `PreparedQueryStream`, and the `query_cli` backend-selection tests.
- Service behavior remains `LiveObserved`, while Cloud Terrastodon and local
  no-daemon paths use the in-process published-index runtime/session story.
- Validated with:
  - `cargo check`
  - `cargo test query_runtime --quiet`
  - `cargo test query_cli --quiet`
  - `cargo test daemon::tests --quiet`
  - `tracey query validate --deny warnings`

### Phase 3: Unify Query Execution Around Indexed Query Backends

Status:
- Complete

Objective:
- Remove the current split where live queries row-crawl while published-index
  queries use the indexed matcher.

Tasks:
- Refactor `LiveDriveState` to answer queries using the in-memory index cache it
  already rebuilds.
- Reuse or extract the search-index query path so both published-index and
  live-backed querying share the same matching engine where appropriate.
- Preserve filtering, drive scoping, deleted-state handling, and result limits.
- Preserve cancellation checks while moving away from row-crawl query logic.
- Keep the old live-query path only as long as needed to compare behavior or
  stage the migration safely; remove it once the indexed path is validated.

Definition of done:
- Live daemon queries no longer depend on `request.query.matches(...)` over
  every projected path as the primary execution path.
- Query behavior remains compatible with existing semantics for limits,
  filtering, and deleted-state handling.
- Cancellation tests still pass.
- Tracy/Tracey instrumentation is updated if the refactor changes the important
  query-time phases.
- The daemon query surface remains usable throughout the migration.

Progress note:
- `LiveDriveState::query_with_cancel()` now routes live queries through indexed
  matching over `current_index_bytes_cache` instead of scanning projected paths
  with `request.query.matches(...)`.
- Live query cache rebuild now has a cancel-aware path so pre-query projection
  and index rebuild work can stop cleanly.
- `src/query/search_index_query.rs` now exposes a shared parsed-search-index row
  visitor, and `LiveDriveState` uses that shared path instead of duplicating
  parsed-index materialization logic locally.
- Focused regression coverage now exists for:
  - successful matching through the live query cache
  - cancellation before indexed query execution begins
  - limit handling after deleted-row filtering
  - `only_deleted` semantics over indexed live rows
  - `.teamy_mft_rules`-driven filtered row classification
  - degraded error reporting for corrupt cached live index bytes
  - directory and exact-file scope filtering against real canonicalized temp
    paths
  - `RequestInvalid` mapping for unresolvable scope paths
  - `Degraded` mapping for malformed discovered filter-rule files
- `LiveDriveState` now preserves the chained `eyre` error text when converting
  indexed live-query failures into `MachineError`, which makes degraded/request-
  invalid responses more diagnosable for callers.
- This phase now satisfies its definition of done from the current evidence.

### Phase 4: Add An In-Process Published-Index Worker Facade

Status:
- Complete

Objective:
- Provide a reusable in-process repeated-query cache for callers like Cloud
  Terrastodon without requiring RPC or live observation.

Tasks:
- Introduce a small public or crate-internal facade that spins up drive workers
  in `PublishedIndexOnly` mode.
- Ensure repeated queries reuse cached drive state instead of re-opening and
  reparsing search indexes every time.
- Expose a query API that preserves graceful cancellation semantics.
- Refactor the `query` CLI `!self.daemon` path so it uses the same top-level
  row-streaming/cancellation surface as the daemon path, differing only in
  backend/runtime selection.
- Decide whether this facade should be presented as a `QuerySession`, an
  in-process daemon handle, or another clearly named runtime object.
- If the old direct `DiskQueryExecutor` path is still retained for debugging,
  mark it as transitional in code/comments and document the planned removal
  point.

Definition of done:
- A caller can create one in-process handle and issue multiple queries without
  repeated one-off setup costs.
- The handle can be cancelled or dropped cleanly.
- The code path does not depend on USN observation or overlay flushing.
- `query --no-daemon` no longer has to build a completely separate result
  collection flow from `query --daemon`.
- Tests demonstrate repeated-query reuse and cancellation behavior.
- CLI behavior remains stable from the user’s perspective unless an intentional,
  documented change is chosen.

Progress note:
- `QuerySession::published_index_only()` exists and is reusable from library
  code.
- Cloud Terrastodon already uses one session per software-list operation.
- `QuerySession::collect_rows_with_cancel(...)` now gives the cached published-
  index path a best-effort cancellation-aware surface.
- `cloud-terrastodon/crates/software/src/lib.rs` now also exposes
  `list_software_counts_with_cancel(...)` so higher-level command code can
  consume the session cache with an explicit cancellation token.
- The session now shares the same parsed-search-index visitor used by the live
  query path, reducing duplication between repeated-query and live-backed
  indexed matching.
- `QueryRuntime::PublishedIndexOnly` now builds its local streamed query path on
  top of `QuerySession::published_index_only()` instead of the older direct
  `DiskQueryExecutor` path, so the CLI/shared runtime surface and Cloud
  Terrastodon now converge on the same in-process published-index cache
  behavior.
- The session-backed local runtime path now has its own Ctrl+C flag forwarder
  and explicit cleanup so it preserves the same graceful cancellation shape as
  the daemon-backed runtime path.
- `QuerySession` now also exposes `visit_rows_with_cancel(...)` and
  `count_rows_with_cancel(...)`, so the facade is no longer limited to bulk row
  collection.
- The local runtime path now streams rows from the session visitor directly
  instead of buffering all local query results before they reach
  `QueryRowStream`.
- The current session still caches mapped published index files, not a fully
  retained parsed query-time index representation.
- This phase now satisfies its definition of done from the current evidence.

### Phase 5: Integrate Cloud Terrastodon Explicitly

Status:
- Complete

Objective:
- Switch Cloud Terrastodon from accidental one-off disk queries to an explicit
  repeated-query cache choice.

Tasks:
- Update the `cloud-terrastodon` software query flow to construct and use the
  new in-process repeated-query handle.
- Install or reuse graceful Ctrl+C cancellation when running software list or
  similar repeated-query commands.
- Keep the integration explicit in code so it is obvious that Cloud Terrastodon
  chose the in-process cache mode intentionally.
- Measure or at least log enough information to confirm that repeated queries
  are using the persistent path.

Definition of done:
- Cloud Terrastodon no longer loops over fresh `QueryArgs::collect_rows()` calls
  for repeated software detection queries.
- Cancellation remains graceful.
- The integration is documented in code or notes so future readers understand
  why it is not using the old implicit `no-daemon` path.

Progress note:
- `cloud-terrastodon/crates/software/src/lib.rs` now creates one
  `QuerySession::published_index_only()` and reuses it across its repeated
  software queries.
- The software helper now uses `QuerySession::count_rows_with_cancel(...)`
  instead of collecting rows only to take their length.
- The integration choice is now called out directly in code so future readers
  can see that software detection intentionally chooses the in-process
  published-index cache path instead of an accidental one-off `--no-daemon`
  query flow.
- `cloud-terrastodon` `software list` now wires `tokio::signal::ctrl_c()` into
  the cancel-aware software helper, so graceful cancellation is preserved at the
  top-level command path instead of only in the library helper.
- This phase now satisfies its definition of done from the current evidence.

### Phase 6: Hardening, Coverage, And Cleanup

Status:
- Complete

Objective:
- Finish the refactor with enough tests, docs, and coverage that the new worker
  model is stable and resumable.

Tasks:
- Add regression tests for:
  - worker mode selection
  - warmup policy differences
  - published-index-only repeated queries
  - live-observed query execution
  - cancellation during query
  - fallback behavior when live refresh degrades
- Re-run Tracey validation and address any newly uncovered or unmapped areas.
- Update docs and notes to reflect the final architecture.
- Capture any deferred follow-up work separately instead of leaving it implicit.
- Remove any transitional adapters or deprecated internal paths that are no
  longer needed after the replacement path is proven.

Definition of done:
- Relevant tests pass.
- Tracey validation passes.
- This plan file is updated to show completed phases, remaining debt, and any
  intentionally deferred work.
- A fresh agent can tell from the plan and docs whether the overall goal is
  complete.
- Any compatibility shims left in place are either intentionally documented as
  long-term or removed before the phase is closed.

Progress note:
- Added worker-layer warmup/runtime-mode regressions in `src/machine/daemon.rs`
  covering both sides of the current policy split:
  - `published_index_only_worker_ignores_warm_requests`
  - `warm_next_drive_worker_creates_live_observed_worker_with_snapshot_fallback`
- Moved local published-index stream production into
  `QuerySession::spawn_stream(...)`, which makes the steady-state boundary
  explicit: `QuerySession` owns local repeated-query/session-backed production,
  while `QueryRuntime` stays responsible for backend selection and shared
  top-level one-shot orchestration.
- Added `published_session_spawn_stream_emits_matching_rows` to prove the new
  session-owned local stream path still produces query rows correctly.
- Documented in code that `DiskQueryExecutor` is retained as the specialized
  direct local streaming helper with explicit `QueryFilterBehavior` control
  rather than the default CLI/Cloud-Terrastodon path.
- Added `query_cli` tests that prove the shared top-level query surface still
  maps flags to backends intentionally:
  - default and `--no-daemon` use `PublishedIndexOnly`
  - `--daemon` uses `DaemonRpc`
  - conflicting flags fail before profile/config work
- Those tests complement the earlier lower-level `DriveWorkerState` coverage by
  proving the actual worker loop honors `PublishedIndexOnly` warm no-op
  behavior and that daemon warmup still creates `LiveObserved` workers.
- Validated with:
  - `cargo fmt --all`
  - `cargo check`
  - `cargo test daemon::tests --quiet`
  - `cargo test query_cli --quiet`
  - `cargo test query_session --quiet`
  - `cargo test query_runtime --quiet`
  - `cargo test disk_query_executor --quiet`
  - `cargo test published_session_spawn_stream_emits_matching_rows --quiet`
  - `cargo test published_index_only_worker_ignores_warm_requests --quiet`
  - `cargo test warm_next_drive_worker_creates_live_observed_worker_with_snapshot_fallback --quiet`
  - `tracey query validate --deny warnings`
- `cargo clippy --all-targets --all-features -- -D warnings` and
  `check-all.ps1` still fail only on the same repo-wide pre-existing lint debt
  outside this slice.
- This phase now satisfies its definition of done from the current evidence.

Completion note:
- The final hardening pass now covers:
  - worker runtime-mode selection and warmup-policy differences
  - repeated-query session reuse and local session-backed stream production
  - live-observed indexed query execution and cancellation/degraded behavior
  - top-level CLI daemon-vs-no-daemon backend selection semantics
  - the retained `DiskQueryExecutor` specialization via explicit
    `QueryFilterBehavior` control
- Tracey validation remains clean.
- The only remaining items are explicitly deferred follow-ups rather than
  incomplete core refactor work.

## Recommended Implementation Order

1. Phase 1: specify the target runtime model
2. Phase 2: split orchestration from runtime mode
3. Phase 3: unify query execution around indexed backends
4. Phase 4: add the in-process published-index worker facade
5. Phase 5: integrate Cloud Terrastodon explicitly
6. Phase 6: harden, validate, and clean up

This order is intentional:

- Phase 1 prevents architectural drift while refactoring.
- Phase 2 creates the seam needed for both service and in-process behavior.
- Phase 3 fixes the daemon’s current data-plane mismatch before other callers
  build on it.
- Phase 4 gives Cloud Terrastodon the right primitive.
- Phase 5 consumes the primitive only after it is stable.
- Phase 6 keeps the result maintainable.

## Plan Maintenance Protocol

This file must be updated after every meaningful work session, not only at the
end of the overall project.

At minimum, each update must:

- revise `Current Status`
- mark which phase is complete, in progress, or not started
- record any architectural decision that changed the plan
- record which files, tests, or Tracey commands were touched
- state the next recommended step

When a phase completes, add a short completion note under that phase covering:

- what changed
- what validated the change
- what follow-up remains, if any

Do not rely on chat history as the handoff artifact. This file is the handoff
artifact.

## Open Decisions

- Final public naming:
  - `QuerySession`
  - `PublishedQueryWorker`
  - `InProcessDaemon`
  - another facade name that clearly communicates “persistent repeated-query cache”
- Whether `PublishedIndexOnly` should live entirely inside `daemon.rs` at first
  or be extracted into smaller worker/runtime modules during Phase 2.
- Whether Cloud Terrastodon should consume the repeated-query handle directly or
  through a thin `teamy-mft` library helper tailored to software-detection
  workflows.
- Whether any service CLI flags or config toggles should expose runtime mode
  choices explicitly, or whether the mode split should remain internal for now.
- Whether we need an additional explicit debug flag later for “force cold disk
  path” once `--no-daemon` is backed by the reusable in-process runtime instead
  of one-off `DiskQueryExecutor` construction.

## First Concrete Slice

1. Create `docs/spec/product/daemon-worker-runtime.md`.
2. Capture requirements for:
   - worker runtime modes
   - query serialization vs mutation
   - warmup policy
   - published-index-only behavior
   - cancellation expectations
   - fallback and degraded-state behavior
3. Map the spec to the current daemon/worker implementation.
4. Run:

```powershell
tracey query validate --deny warnings
tracey query unmapped --path src/machine/daemon.rs
```

5. Update this plan file’s `Current Status` section with the spec path, mapping
   result, and the exact next refactor entry point for Phase 2.
