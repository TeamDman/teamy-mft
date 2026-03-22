## Plan: Fast Query Engine Redesign

Replace the current Nucleo-based fuzzy query path with a purpose-built fast first-pass engine that supports only two operations: `contains_case_insensitive` and `ends_with_case_insensitive`. Pair that engine rewrite with a storage redesign toward zero-copy indexed data on disk, using `zerotrie` where it fits the string dictionary layer, so query time is dominated by compact scans and simple rule evaluation instead of fuzzy-matcher setup, path cloning, and full-path heap allocation.

**Progress Update**
1. Completed: the fast query path no longer uses Nucleo. Matching now uses the custom rule engine limited to `contains_case_insensitive` and `ends_with_case_insensitive`.
2. Completed: query parsing was moved into `src/query/` and split into dedicated types such as `QueryPlan`, `QueryGroup`, `QueryRule`, and `QueryNeedle`.
3. Completed: CLI query inputs now normalize both repeated positional arguments and `|`-separated segments into the same OR-of-ANDs query plan.
4. Completed: the sync pipeline was cleaned up to use `Vec<DriveSyncInfo>` worklists instead of `BTreeMap<char, ...>` in the main flow, reducing incidental complexity while the query work was landing.
5. Completed: baseline validation is green again; `./check-all.ps1` passes after the refactors.
6. Completed: the search-index loader now exposes borrowed `SearchIndexPathRowView` values from the memory map, and the query path allocates owned `IndexedPathRow` values only for matches.
7. Completed: the search-index format is now versioned, and newly written index rows store both the display path and a normalized lowercase path so query evaluation can avoid per-row lowercase work.
8. Completed: the query path now consumes pre-normalized row views from the current index format, and stale legacy indexes are rejected with an explicit prompt to rerun `sync index` for the affected drives.
9. Completed: the on-disk index format has moved from row-oriented full-path payloads to a segment dictionary plus parent-chain node table, and result paths are reconstructed only when needed.
10. Completed: `zerotrie` is now part of the search-index data area as the serialized normalized segment dictionary.
11. Completed: the format has a deterministic small-index byte snapshot test that guards parser changes against silent `SEARCH_INDEX_VERSION` drift.
12. Completed: query parsing now performs eager validation for Windows-invalid path characters while preserving the recognized query syntax and drive-designator forms.
13. Completed: query-time search-index reads now use a trusted parse path that skips the expensive full-table validation passes while keeping the validated parse path for tests and generic callers.
14. Completed: query-time segment matching now operates on normalized segment text and avoids eager display-string UTF-8 decoding in the hot path.
15. Completed: common extension-style suffix queries such as `.jar$` now use persisted extension suffix postings in the on-disk search index instead of rebuilding an in-memory suffix map per query parse.
16. Completed: the search-index format was intentionally bumped to version 5 for persisted extension suffix postings, and then to version 6 for persisted trigram contains postings, so stale indexes are rejected and rebuilt rather than carrying compatibility for older layouts.
17. Completed: Tracy span naming and coverage are now good enough to distinguish parse-body-prefix, segment decode, extension decode, postings lookup, row materialization, and output costs in one capture.
18. Completed: generic `contains_case_insensitive` rules of normalized byte length at least 3 now use persisted trigram (`n = 3`) postings to narrow candidate segment IDs before exact segment matching and row-postings collection.
19. Completed: short contains needles below trigram length still fall back to the existing linear segment scan, keeping the first contains-oriented index small while preserving behavior.
20. Current profile target: re-run Tracy against representative contains and mixed contains-plus-suffix queries to measure how much of `match_query_rule_against_segments` and `parse_search_index_segments` has been eliminated by the trigram path.

**Steps**
1. Completed Phase 1: narrowed the product boundary in the query layer so the indexed fast path is explicitly a rough first pass supporting only `contains_case_insensitive` and `ends_with_case_insensitive`.
2. Completed Phase 1: defined the internal rule model in `src/query/`, with parsing and normalization logic that produces OR-of-ANDs query plans from both repeated positional inputs and `|` syntax.
3. Completed Phase 1: removed Nucleo from the execution design and replaced matcher setup, injection, tick loops, and snapshot reads with direct rule evaluation over indexed records.
4. Partially completed Phase 1: adopted the request-object style structurally by splitting query logic into dedicated types and modules, but the explicit `IntoFuture` request-object orchestration has not been introduced yet.
5. Completed Phase 2: improved zero-copy reads by iterating borrowed row views from the memory map in `src/search_index/load.rs`, while keeping the old owned `rows()` path as a compatibility wrapper.
6. Completed Phase 2: redesigned the on-disk `.mft_search_index` format into a versioned layout and then into a segment-dictionary plus parent-chain representation, rejecting stale indexes and forcing rebuilds rather than carrying legacy read paths indefinitely.
7. Completed Phase 2: introduced `zerotrie` in the immutable data area as the serialized normalized segment dictionary.
8. Completed Phase 2: removed the row-oriented full-path serialization in favor of compact segment reuse and deferred path reconstruction for matched rows.
9. Remaining Phase 3: rework the execution pipeline around smaller owned request stages or task stages so load, decode, filter, and materialization are more explicitly modeled; this remains structurally desirable but is no longer the top latency lever.
10. Completed Phase 3: Tracy instrumentation now distinguishes index open time, search-index header/body-prefix decode, segment decode, extension decode, rule-to-segment scan time, posting collection time, candidate materialization time, and output time.
11. In progress Phase 4: verify behavior and latency against representative contains and suffix queries, compare with the previous implementation, and remove any leftover compatibility paths once the new storage format is in place.
12. Completed Phase 4: added a deterministic small-index snapshot-style regression test that writes real `.mft_search_index` bytes and fails format-parser changes unless `SEARCH_INDEX_VERSION` is intentionally incremented.
13. Completed Phase 4: replaced one important class of full segment scans with direct query-to-segment candidate lookup for common extension-style suffix queries using persisted extension postings.
14. Completed Phase 4: added persisted trigram candidate lookup for generic `contains_case_insensitive` rules whose normalized needles are at least 3 bytes long, using trigram postings to produce candidate segment IDs before exact rule checks.
15. Completed Phase 4: the query planner now uses suffix postings for extension-like `ends_with` rules, trigram postings for contains needles of length at least 3, and the existing segment scan as the fallback for shorter contains needles.
16. Next concrete step: capture fresh Tracy profiles for representative contains and mixed-rule queries, then decide whether the remaining parse/decode costs justify further index compaction, segment decode deferral, or a two-character candidate path.
17. Deferred consideration: adding more Tokio task staging or work stealing may still help orchestration later, but current Tracy captures show the root bottleneck is algorithmic candidate generation inside each drive, not lack of coarse-grained scheduling.

**Relevant files**
- `g:/Programming/Repos/teamy-mft/src/cli/command/query/query_cli.rs` — current indexed query entry point; now consumes segment iterators from the mapped index and materializes owned rows only for matches.
- `g:/Programming/Repos/teamy-mft/src/query/query_plan.rs` — owns the OR-of-ANDs query normalization, eager query-input validation, and the plan-level segment matching entry point.
- `g:/Programming/Repos/teamy-mft/src/query/query_group.rs` — defines one AND-group within a query plan.
- `g:/Programming/Repos/teamy-mft/src/query/query_rule.rs` — defines the supported rule kinds for the new fast query engine.
- `g:/Programming/Repos/teamy-mft/src/query/query_needle.rs` — owns the case-insensitive matching primitives used by the fast rule engine.
- `g:/Programming/Repos/teamy-mft/src/search_index/load.rs` — exposes zero-copy row views over the current mapped segment-based index format and rejects stale index versions with a rebuild prompt instead of maintaining legacy parsing code.
- `g:/Programming/Repos/teamy-mft/src/search_index/format.rs` — current versioned disk format header; the data area is now segment-based rather than full-path row based.
- `g:/Programming/Repos/teamy-mft/src/search_index/search_index_bytes.rs` — current center of the segment dictionary, parent-chain node encoding, persisted extension and trigram postings, byte parsing, and snapshot regression coverage.
- `g:/Programming/Repos/teamy-mft/notes/search-index.md` — existing long-term indexing ideas; should be updated to reflect the new immediate focus on two-rule matching and zero-copy storage.
- `g:/Programming/Repos/teamy-mft/notes/parallel work.md` — still relevant for task decomposition and scheduling once the matcher is removed.
- `d:/Repos/Azure/Cloud-Terrastodon/crates/azure/src/resource_groups.rs` — request-object and `IntoFuture` style reference.
- `d:/Repos/Azure/Cloud-Terrastodon/crates/azure_devops/src/azure_devops_agent_pool_entitlements_for_project.rs` — parameterized request-object example to mirror at the API edge.
- `g:/Programming/Repos/icu4x/utils/zerotrie/Cargo.toml` — confirms available `zerotrie` crate version and features to plan against.
- `g:/Programming/Repos/icu4x/utils/zerotrie/examples/first_weekday_for_region.rs` — concrete example of compact immutable byte-backed trie data.

**Verification**
1. Run `cargo check` and `cargo check --features tracy` after each major refactor step.
2. Re-run `cargo run --release --features tracy -- query "flower .jar$" --debug` or `./run-tracing.ps1 query "flower .jar$"`, and compare Tracy captures before and after each candidate-index change.
3. Add focused tests for parsing and evaluating `contains_case_insensitive` and `ends_with_case_insensitive`, including mixed-case inputs, trigram candidate reduction behavior, and deleted/non-deleted filtering interactions.
4. Compare rough-pass output counts against the current implementation for representative contains and suffix searches so the behavior change is explicit and intentional.
5. Benchmark index loading and query execution separately so the impact of zero-copy storage can be distinguished from the impact of dropping Nucleo.
6. Keep `tests/cli_fuzzing.rs` green as the CLI shape evolves, since the query command now depends on repeated positional arguments and OR-group normalization.
7. Keep the snapshot byte test current only when the format change is intentional and paired with a `SEARCH_INDEX_VERSION` bump.

**Decisions**
- Nucleo is out of scope for the new fast path.
- The supported operations are intentionally limited to `contains_case_insensitive` and `ends_with_case_insensitive`.
- The fast query path is a rough first pass, not a full advanced filtering engine.
- `zerotrie` is part of the storage redesign plan, especially for compact immutable string representation.
- Query inputs should be rejected early when they contain Windows-invalid path characters outside the intentionally supported query syntax.
- The request-object plus `IntoFuture` style from Cloud-Terrastodon remains the orchestration pattern.
- Zero-copy and normalized storage are now first-class performance goals alongside CPU saturation.
- Extension suffix postings remain a first-class exact index and will not be folded into a generic contains-only candidate path.
- The first contains-oriented candidate index will use normalized trigrams (`n = 3`).
- Contains needles shorter than 3 characters will initially use the existing linear segment scan rather than carrying a bigram or unigram index before the profile justifies it.

**Further Considerations**
1. `contains_case_insensitive` is usually harder to accelerate with trie structures than `ends_with_case_insensitive`, so the disk format should continue as a hybrid design: exact/special-case postings where they are selective, plus bounded n-gram candidate indexes for broader substring work.
2. The new rule engine now scans normalized path segments instead of full normalized paths, and both suffix-oriented and trigram candidate acceleration are now persisted; the next storage optimization should be driven by fresh profiling rather than assumed ahead of the new data.
3. If backwards compatibility of `.mft_search_index` matters, plan an explicit format version bump and migration story instead of trying to keep the old row layout partially alive.
4. The current Tracy captures show that parallel per-drive loading is no longer the main issue; the dominant remaining work is per-query segment-table decode and per-rule full segment scans within each drive, so the next material win is algorithmic candidate reduction rather than more coarse-grained parallelism or scheduler changes.
5. If we later decide to use Tokio more aggressively, it should follow the trigram candidate-index work so the runtime is stealing useful smaller tasks instead of distributing the same full-table scans more widely.
6. A future bigram index remains an option for two-character contains queries, but it should be justified by real usage and profiling because its postings will be much broader than the trigram case.
