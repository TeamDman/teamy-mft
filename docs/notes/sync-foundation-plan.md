# Sync foundations and progress UI

**Plan status:** Active

**Primary implementation root:** `TeamDman/teamy-mft`, with dependency work in the named repositories below

**Last updated:** 2026-11-08

**Intent audit:** Passed against the user's 7 November cross-repository request and subsequent plan-location approval

## How to update this plan

- `[ ]` Not started; `[~]` In progress; `[x]` Complete; `[!]` Blocked.
- Update each task's status and evidence together. Record revisions, commands and remaining limitations under the affected task.
- Complete a phase only after its acceptance checks pass. Keep this file current rather than appending a historical work log.
- Current implementation owners: dependency merge/qualification, template qualification, and the independently reproducible MFT logging defect may proceed in parallel. Scheduler redesign and UI implementation follow their decision gates.

## User guidance ledger

| ID | Active guidance | Coverage |
| --- | --- | --- |
| U1 | Keep raw `.mft` snapshots and the current owned-buffer/fixup approach. Do not pursue mmap or eagerly fix records before snapshot publication; the search index is the efficient query representation. | Foundation; task 1 |
| U2 | Explain the roles of Tokio and Rayon; consider a clearer per-drive MFT-then-index workflow, possibly typed work in a Tokio `JoinSet`. Preserve overlap between acquisition and indexing across drives. | Task 4; gate G2 |
| U3 | Separate acquisition, index construction and orchestration ownership; improve awkward `invoke_for_mft_path`/`invoke_for_mft_file` naming and test-only generic scheduling complexity. | Tasks 3–4 |
| U4 | Find unused APIs and repeat the analysis after removing dead callers; explain why `cargo check` and rust-analyzer missed them. | Task 3 |
| U5 | Reduce excessive qualified names and establish practical executable lint checks where possible. Assess existing tooling before building a `syn` control-flow analyzer or using the local LLM decider for judgment. | Task 3; gate G3 |
| U6 | `teamy-llm decide` help was present but obscured by many global options. Evaluate help presentation without assuming an absent-help parser bug. | Task 7 |
| U7 | Use the user's existing task workspace and references: MFT, LLM service, docs, Facet, facet-format, Figue, tracing, cancellation, CLI template and Cloud Terrastodon. Preserve independent repository work. | Tasks 1–7; constraints |
| U8 | Resolve the immediate facet-format pull conflict, understand the divergence, and determine whether another crate release is necessary. | Task 1 |
| U9 | Qualify the latest compatible Facet ecosystem in `teamy-rust-cli` first, then propagate deliberately to MFT. Cloud Terrastodon/work-machine development may have advanced shared crates. | Task 2; gate G1 |
| U10 | Find historical MFT grid UI work in Git history/branches or `storage-usage`; use the linked PR comment and screenshot as prior art. | Task 5 |
| U11 | Work toward an inline Ratatui sync display at the bottom of the terminal, retaining diagnostic logs above it without entering the alternate screen. Study Cloud Terrastodon's Excalidraw interface. | Tasks 5–6 |
| U12 | Use independently colored upper/lower half-cell pixels for a denser grid. Show per-drive volume/MFT regions and animate meaningful work. | Tasks 5–6; gate G4 |
| U13 | Emit structured observable progress from MFT/index work; keep diagnostics usable and support both CLI and daemon execution. | Tasks 4 and 6; gate G4 |
| U14 | Investigate issue #14: daemon sync can fail silently or omit index logs; restarting the daemon helped. Keep lifecycle/state problems distinct from established logging defects. | Task 3; gate G5 |
| U15 | Preserve reusable findings in Teamy Docs and organize implementation/dependency follow-ups. | Tasks 5 and 8 |
| U16 | Use `docs/notes/sync-foundation-plan.md` as the living cross-repository plan. | This file |
| U17 | Follow TeamyStyle, discoverable type files, typed boundaries and minimal indirection. Preserve dirty work and staged selections. | All tasks |
| U18 | Limit rust-analyzer in the existing multi-repository workspace to MFT after adding Vox. | Foundation; explicit workspace-setting exception |
| U19 | Let `MachineConfig` own loading and access its `sync_dir` field directly rather than using free loading helpers. | Foundation; task 4 |
| U20 | Extract `MachineCacheSyncRequest` and the existing result into domain-type files, moving cache-sync execution out of the daemon. | Task 4 |
| U21 | Pass the Copy overwrite policy by value; inline the single-use tray config loader and handle its result with one match. | Foundation; task 4 |
| U22 | Unify the duplicate drive records as `DriveArtifactInfo`; `DriveArtifactInfoCollection::resolve` must own a map, with the directory supplied as an argument. | Task 4 |
| U23 | Borrow the original artifact collection through sync planning and execution, avoid redundant collection clones, and iterate its keys instead of building a requested-drive vector. | Task 4; container ownership superseded by U25, cloning constraint retained |
| U24 | Fix the reproduced query failure caused by an inactive named rules file, and correct the misleading missing-index diagnosis. | Task 3.3 |
| U25 | Let `SyncWork` own one map pairing each drive's artifact info with `SyncDecision::{Skip, Index, MftAndIndex}`. Move records into the map, remove separate drive sets, retain skipped records for reporting and keep decisions unchanged during execution. Omit the unused MFT-only variant. | Task 4 |
| U26 | Move the shared base search-index requirement check onto `DriveArtifactInfo` and call it from both query loaders. | Task 3.3 |

## Intent audit evidence

- **Extraction:** Read the complete current cross-repository request. U1–U17 retain its immediate merge priority, tentative scheduler suggestions, deferred UI direction, help correction, source references and approval of this file. U18–U23 record the subsequent workspace and domain-refactor directions, including the final accepted artifact names, `resolve` spelling, map ownership and borrowed collection lifetime. U24 covers the subsequent reproduced query failure and authorized fix. U25 records the accepted decision-map design, superseding the earlier collection ownership while preserving the constraint against redundant clones. U26 places the shared index requirement check on its artifact owner.
- **Traceability:** Every ledger row names a task, gate or constraint. The immediate merge and template checks precede application dependency adoption; independent logging repair does not depend on a new UI.
- **Adversarial check:** A suggested `JoinSet` is not a settled design; no whole-volume scan or actual corruption is inferred from the old screenshot; a restart workaround does not establish issue #14's cause. Publication, runtime compatibility and source inspection remain separate evidence. Artifact records move into one decision map; execution and publication borrow it, and no-op selections remain valid. Decisions describe intent rather than progress, preserving fresh-snapshot identity for journal cursors and sidecars. Blocking tasks own their records rather than capturing borrowed paths. U18 explicitly authorizes the narrow workspace-setting change.
- **Source limitation:** Earlier conversation details are partly available through a compacted summary. Completed earlier refactors are foundation, not new implementation requirements. The current long request and its plan approval were read directly.

## Foundation and constraints

- MFT revision `54bd17a` extracts `SyncWork` and preserves bounded CPU work and acquisition/index overlap. The tree was clean at the start of this work.
- Existing snapshot publication, DACL preservation, cancellation, full-sync writer guard and artifact consistency must survive refactoring.
- Raw snapshots are fixed up in an owned buffer before indexing. Mapped search-index reads are a separate existing design; rejecting raw-snapshot mmap does not remove them.
- Do not acquire stdout inside reusable operations or let UI drawing affect machine output. Reader/indexer progress types must not depend on Ratatui.
- Preserve unrelated changes in each repository and the user's workspace folders. U18's requested rust-analyzer `linkedProjects` setting selects MFT only; no other workspace reshaping or unrelated discovery-tool repair is included.
- Do not start/stop a real daemon, read live privileged volumes or change service/security settings merely to qualify synthetic scheduling or UI work.
- No crate publication or push has been performed for this work. Prepare and review any required release as a separate concrete step.
- Use project-relative paths and public immutable source links in shared documentation; do not persist machine inventories or raw diagnostic captures.

## Decision gates

| Gate | Decision needed | Evidence required |
| --- | --- | --- |
| G1 | Which mutually compatible published versions can MFT adopt? | Verify exact companion pins and enabled features, including Vox; qualify template and protocol consumers. A local patch is not a published release. |
| G2 | Which owner and scheduler best express per-drive phases? | Keep bounded CPU work, responsive Tokio tasks, nested Rayon algorithms, cancellation, drain-on-error and read/index overlap. Compare direct Rayon dispatch with a Tokio completion channel against current blocking-thread bridge; a `JoinSet` alone does not supply CPU isolation. |
| G3 | Which lint rules can be enforced without noisy exceptions? | Probe compiler/Clippy behavior on small representative fixtures. Public API compatibility requires an explicit project decision; lexical references alone are not reachability. |
| G4 | What typed progress and transport contract feeds the UI? | Distinguish planned extents, successful writes and publication; define IDs, ranges, bounded/coalesced delivery, terminal outcomes and non-TTY behavior before animation. |
| G5 | What causes the restart-sensitive daemon behavior? | Reproduce with correlated synthetic RPC logs and worker lifecycle observations. A missing tracing span proves a logging defect, not the entire intermittent failure. |

## Execution

### [x] 1. Resolve facet-format and establish release state

- Local `teamy-main` tip `bb3720b` and incoming release parent `8c49927` have identical tree `51c6b084dfd8507af015110beda18fb93badf2a1`. Divergent commit ancestry caused conflicts despite identical pre-release contents.
- Incoming `55dd746` adds the generic `Cow<T>` decoding fix, its regression tests and independent package version updates. The merged XML parser was already present on both sides.
- Resolved the 23 conflicted files using the incoming content after checking those identities. The resulting index tree exactly equals incoming tree `5383e8508a7e6ecaa760b906330acaefe1704795`; this retains all local code.
- Completed local merge `03941a0`; checkout is clean. All affected Format/JSON/CSV/Value/DOM/XML/XML-node library, integration and documentation suites pass: `cargo test --locked -p teamy-facet-format -p teamy-facet-json -p teamy-facet-csv -p teamy-facet-value -p teamy-facet-dom -p teamy-facet-xml -p teamy-facet-xml-node --quiet`. The initial offline attempt needed an uncached locked dependency; downloading it resolved that limitation.
- Registry versions are already published and unyanked. All seven published package manifests and all 420 source files match the checkout after line-ending normalization, with no extra checkout source files. No new format release is needed. The merge has not been pushed.

### [~] 2. Qualify the template, then prepare MFT dependency adoption

#### [x] 2.1. Qualify the template integration

- The template was clean but diverged: local `d3e304a` contains eight terminal/Windows foundation commits; existing `origin/main` at `58d9418` updates dependencies. Merge these changes instead of overwriting local UI work.
- Intended published set: Facet facade/error/default rc.8; core/reflect/pretty rc.7; Format/JSON/CSV rc.10; Figue 6.0.0-rc.6 with attrs rc.4; cancellation 0.8.1; tracing 0.2.1. Verify packages individually.
- Template qualification complete in local merge `2c90b54`, preserving all eight terminal/Windows foundation commits. `check-all.ps1` passes nightly formatting, strict all-target/all-feature Clippy, 14 lint-policy fixtures, all-feature build and 45 tests. Two pre-existing protocol-test Clippy failures were repaired using `Vec::as_slice`. Root and leaf help routes pass. No push.

#### [ ] 2.2. Qualify compatible Vox releases and adopt them in MFT

- MFT currently pins Facet rc.7, JSON rc.9, Figue rc.5 and Vox/types/stream rc.9. Cancellation 0.8.1 is already adopted with async/I/O features.
- Published Vox rc.9 pins the older Facet facade exactly. Qualify and release an affected compatible Vox set before claiming that MFT can use the newer published graph; inspect generated code and IPC contracts, not just dependency resolution.
- MFT adoption checks: all-target checking, strict all-feature Clippy, full appropriate tests, CLI help/parsing and daemon wire compatibility. Do not leave speculative incompatible manifest pins behind.
- Next dependency work is the Vox compatibility/release slice. MFT manifest pins have deliberately not been changed to an incompatible published graph; the remaining application upgrade is not complete.

### [~] 3. Remove dead sync APIs and repair observable defects

#### [x] 3.1. Clean up unused APIs, repair index log correlation and strengthen checks

- Removed `SyncIndex::invoke` and `invoke_for_mft_file`, which had no in-repository callers. Their public export made them externally reachable for Rust's `dead_code` analysis. Inlined the remaining single-use row/output helpers, narrowed the internal type to the sync module, and named its remaining operation `build`.
- A native probe confirms that entering a tracing span before `Rayon::install` does not transfer it to the pool thread. The daemon filters forwarded events by correlation ID, so index diagnostics lose the sync association.
- The scheduler now installs the captured subscriber and enters its RPC span inside the Rayon closure. The correlated-event regression failed before the fix and passes afterward; all five focused scheduler tests pass. Output completion is logged only after a successful index write, and row counts use a checked conversion.
- Removed the dead `parse_first_entry_size` → `detect_entry_size` chain, `needs_fixup`, and unused path-collection `entry_count`/`paths_for` accessors. The detector's implementation-only test was removed with the dead implementation.
- Probed `absolute_paths`: 886 sites at the default maximum of two segments, 255 at a maximum of three. Three still catches the requested `DaemonLogWireEvent` example; four would miss it. Adopted maximum three and compiler `unused_qualifications` in the cleaned-up sync CLI only. Broader lint adoption is a separate cleanup; no custom analyzer is needed for these cases.
- Added `dead_code = "deny"` and strengthened `check-all.ps1` to check formatting without rewriting and run strict Clippy on all targets/features. Repaired the exposed existing library/test/example lint failures. Narrow, reasoned subprocess-test expectations retain process isolation without exempting production service management.
- Final `check-all.ps1` passes formatting, strict all-target/all-feature Clippy, all-feature build and 312 tests, with three existing ignored tests. Default-feature tests also pass. `git diff --check` passes. An isolated compile fixture rejects the long qualified path and accepts its imported form plus three-part standard-library paths under the adopted configuration.
- Independent diff review found no blocking correctness issue: explicit IOCP field borrowing, allocation ownership, writer lifetime and publication/cancellation/DACL ordering remain intact. Removed public APIs can break downstream source callers; no new library release was published. Nested Rayon per-record diagnostics are not covered by the high-level index correlation fix.

#### [ ] 3.2. Reproduce restart-sensitive daemon behavior

- Continue issue #14 investigation separately if restart-sensitive missing work remains after log propagation is corrected; do not mark the issue resolved solely from this patch. Use controlled RPC/worker-state observations before making lifecycle changes.

#### [x] 3.3. Isolate inactive rule profiles and preserve accurate query errors

- Reproduced a default query failing on `{GIT_ROOT}` in an inactive named rules file copied outside a Git checkout. The base index existed; a broad error wrapper incorrectly reported it as missing.
- Discovery now skips inactive profiles before reading or parsing their contents. Effective-profile selection owns pattern expansion and compilation; selected and global invalid rules still fail. Administrative discovery retains strict syntax parsing, and stale indexed files skip before filename parsing.
- Both query loaders use `DriveArtifactInfo::require_base_search_index` to distinguish an absent index from a non-file or inspection failure; corrupt indexes and downstream visitor failures retain their own errors. Repair hints use the current `sync --drive-letter-pattern` command.
- `check-all.ps1` passes formatting, strict all-target/all-feature Clippy, all-feature build and 327 tests, with three existing tests ignored. Regressions cover the original rule contents, inactive syntax/UTF-8/glob errors, active/global errors, stale filenames, matching explanations and index error classification. Both original queries succeed against the existing cache using the rebuilt executable. The installed CLI has not been replaced; no rule files or cached indexes were changed.

### [~] 4. Make per-drive scheduling and naming explicit

- Completed ownership changes: `MachineConfig` owns required/optional/client loading, and directory-only callers access its field. The tray uses a single match; the Copy overwrite policy passes by value. `MachineCacheSyncRequest` owns the full-cache operation, with async and blocking execution routes and a separate result file; daemon worker refresh remains in its adapter.
- Unified `PublishedDrivePaths` and `DriveSyncInfo` as `DriveArtifactInfo`. `DriveArtifactInfoCollection::resolve` owns a private `BTreeMap<DriveLetter, DriveArtifactInfo>`. `SyncWork::prepare` consumes that collection and moves each record into one private map alongside its `SyncDecision`. Immutable map access keeps paths, identity and planned work together. Duplicate letters resolve/schedule once in alphabetical order. The former record files and free resolver are removed.
- Preparation now owns the combined preflight policy. `Skip` retains an existing base index and its snapshot/sidecars, indexes a cached MFT when its index is missing, or acquires and indexes when both are absent. `Overwrite` acquires and indexes; `Abort` rejects either existing output before execution. Separate drive sets, collection clones and fallible path lookups are removed. MFT streaming borrows selected records, with individual record clones at worker boundaries; its single-use free stream helper is inlined. Decisions stay unchanged for fresh-only journal cursor capture and sidecar publication, and skipped entries remain available for reporting.
- Decision-map qualification passes `check-all.ps1`: nightly formatting, strict all-target/all-feature Clippy, all-feature build and 328 tests, with three existing tests ignored. Tests cover mixed-drive identity/order/skip retention, all twelve policy/existence combinations, unchanged decisions, sidecar preservation, worker limits, acquisition/index overlap and error draining. Independent review found no blocking issue. No privileged live-volume test was run. Scheduler replacement under G2 remains pending.
- Model acquisition and index-build work as separate domain operations, with `SyncWork` owning orchestration. Cached snapshots can enter the index phase directly; fresh snapshots enter only after successful publication.
- Resolve G2 before replacing the scheduler. Tokio owns waits/IPC and I/O coordination; Rayon currently owns nested parallel fixups, record scans and path resolution. Blocking those CPU loops on async workers would harm responsiveness.
- Consider a typed work/completion enum and `JoinSet` for lifecycle clarity. Avoid extra generic callbacks whose only production caller is fixed. Prefer production-path tests at meaningful effect boundaries.
- Acceptance: two drives demonstrate acquisition/index overlap; worker limits apply to CPU work; errors stop new dependent work while every started acquisition/index task is awaited; cancellation never leaves a background writer behind.

### [x] 5. Identify terminal prior art

- Screenshot source: [storage-usage `42489050`, `mft_summarize.rs`](https://github.com/TeamDman/storage-usage/blob/42489050a20eecf9ef47ec3741dfcec0d469ae73/storage-usage-v2/src/mft_summarize.rs#L883). It colors whole cells. Half-block rendering is the proposed enhancement, not a feature established by that screenshot.
- [Original PR comment](https://github.com/omerbenamram/mft/pull/129#issuecomment-3040830323) later identifies a fragmented-MFT dump bug. Do not treat its bad-entry grid as evidence of genuine corruption.
- No Ratatui history was found in the available local MFT refs; this does not prove that no remote or unreachable work exists.
- Cloud Terrastodon `fd989bd` provides `crates/user_input/src/inline_terminal.rs` and the Excalidraw interactive session: inline stderr viewport, logs above, explicit ownership, acknowledgements and restoration. Its ordinary replay storage is unbounded and must not be copied blindly.
- Template `d3e304a` already contains optional inline-terminal infrastructure and bounded replay. It escapes ANSI/truncates records to width, whereas Cloud Terrastodon's formatter preserves styled multiline output/wrapping. Choose the intended presentation explicitly.

### [ ] 6. Implement typed progress before the inline grid

- Logical/physical read plans already provide MFT extents and completed physical ranges. Report copied progress after `entry.write(writer)` succeeds; report published state only after atomic snapshot publication succeeds.
- Whole-volume dimensions are an additional data requirement: drive validation currently obtains NTFS volume metadata but discards it. Paint known MFT regions within that denominator; do not imply a full-disk scan.
- Define domain progress for lookup, planning, copied ranges, snapshot publication, index parsing/fixups/build/publication and success/failure/cancellation. Use operation/run and drive identity; accommodate out-of-order completions and retry/reset semantics.
- Current daemon sync transport carries logs. Add a closed typed progress stream or an explicitly versioned envelope; do not parse human log messages to reconstruct state. Bound/coalesce delivery so a slow UI cannot stall disk work.
- Build a pure state reducer and half-block renderer, then the CLI inline adapter shared by local and daemon sync. Preserve diagnostic scrollback, stdout results and ordinary non-TTY logs.
- Acceptance: synthetic TestBackend/PTY tests cover tiny/zero dimensions, resize, ordering, bounded queues, disconnects, failure/cancellation, output errors and restoration. Actual Windows Terminal/ConPTY behavior needs explicit host qualification.

### [x] 7. Evaluate LLM help presentation

- Installed `teamy-llm` reports version 0.1.0, revision `6aaae2c`, from a dirty build. Source HEAD reports the same revision but also has dirty work, so their uncommitted contents are not established as identical.
- `decide --help` exits successfully with 120 lines: 74 leaf-help lines followed by 46 global-help lines. The existing `decide --help=short` exits successfully with the 74 leaf lines and omits the appended globals. No inference was run.
- This short-help route is the LLM CLI's adapter, not Figue's command-list mode. A future inherited/global heading or terse/detailed presentation belongs in generated Figue help, preserving global parsing before and after subcommands. It is an optional improvement, not a reproduced absent-help bug. Existing LLM-service dirty work was left untouched.
- Treat an LLM decider as an optional review aid, not authoritative dead-code or lint evidence.

### [x] 8. Reconcile reusable documentation

- Updated Teamy Docs' `facet-ecosystem.md` with qualified template and format evidence, source/registry distinctions and remaining Vox/MFT gates.
- Added grid/inline progress prior art to `terminal-interfaces.md` and its problem-index entry. Added dead-code visibility and qualified-path lint guidance to `code-standards.md`.
- `scripts/build.ps1` passes with mdBook 0.5.4; `scripts/check-links.ps1` validates 3,955 local references in 76 HTML documents. External URLs were not checked by that script. Whitespace checks pass.
- Preserved unrelated existing docs CLI work; added no machine inventories or raw captures. These documentation edits remain uncommitted and unpublished.

## Overall completion and risks

The immediate slice is complete when the facet-format merge and template dependency merge are qualified, release necessity is established, confirmed MFT cleanup/logging repairs pass checks, and the next dependency/UI steps have explicit owners and acceptance criteria. The future progress UI and restart-sensitive daemon investigation remain open until their own tasks pass; a plan does not count as implementing them.

| Risk | Mitigation |
| --- | --- |
| Independent crate suffixes or exact pins produce incompatible Facet types. | Verify actual published manifests and lockfiles; preserve G1. |
| A scheduler rewrite detaches writes or loses CPU bounds. | Preserve production-path drain/overlap tests and G2 before changing dispatch. |
| The UI hides diagnostics or makes success claims before publication. | Typed outcomes, bounded delivery, acknowledgement, checked output and terminal-owner tests. |
| Missing logs are mistaken for the whole daemon failure. | Retain G5 and distinguish confirmed correlation loss from lifecycle hypotheses. |
| Cross-repository cleanup overwrites independent work or invents release status. | Per-repo baseline/diff review, immutable provenance and separate publication checks. |
