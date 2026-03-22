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

**Steps**
1. Completed Phase 1: narrowed the product boundary in the query layer so the indexed fast path is explicitly a rough first pass supporting only `contains_case_insensitive` and `ends_with_case_insensitive`.
2. Completed Phase 1: defined the internal rule model in `src/query/`, with parsing and normalization logic that produces OR-of-ANDs query plans from both repeated positional inputs and `|` syntax.
3. Completed Phase 1: removed Nucleo from the execution design and replaced matcher setup, injection, tick loops, and snapshot reads with direct rule evaluation over indexed records.
4. Partially completed Phase 1: adopted the request-object style structurally by splitting query logic into dedicated types and modules, but the explicit `IntoFuture` request-object orchestration has not been introduced yet.
5. Completed Phase 2: improved zero-copy reads by iterating borrowed row views from the memory map in `src/search_index/load.rs`, while keeping the old owned `rows()` path as a compatibility wrapper.
6. Completed Phase 2: redesigned the on-disk `.mft_search_index` format into a versioned layout and then into a segment-dictionary plus parent-chain representation, rejecting stale indexes and forcing rebuilds rather than carrying legacy read paths indefinitely.
7. Completed Phase 2: introduced `zerotrie` in the immutable data area as the serialized normalized segment dictionary.
8. Completed Phase 2: removed the row-oriented full-path serialization in favor of compact segment reuse and deferred path reconstruction for matched rows.
9. Remaining Phase 3: rework the execution pipeline around smaller owned request stages or task stages so load, decode, filter, and materialization are more explicitly modeled and instrumented.
10. In progress Phase 3: add instrumentation that distinguishes index open time, search-index header/body-prefix decode, segment decode, validation, rule-to-segment scan time, posting collection time, candidate materialization time, and output time so the next Tracy captures show where the remaining latency lives.
11. Remaining Phase 4: verify behavior and latency against representative contains and suffix queries, compare with the previous implementation, and remove any leftover compatibility paths once the new storage format is in place.
12. Completed Phase 4: added a deterministic small-index snapshot-style regression test that writes real `.mft_search_index` bytes and fails format-parser changes unless `SEARCH_INDEX_VERSION` is intentionally incremented.
13. Remaining Phase 4: replace the current full segment scan per rule with direct query-to-segment candidate lookup, starting with suffix-oriented acceleration for common extension queries and then adding a broader contains-oriented candidate index.

**Relevant files**
- `g:/Programming/Repos/teamy-mft/src/cli/command/query/query_cli.rs` — current indexed query entry point; now consumes segment iterators from the mapped index and materializes owned rows only for matches.
- `g:/Programming/Repos/teamy-mft/src/query/query_plan.rs` — owns the OR-of-ANDs query normalization, eager query-input validation, and the plan-level segment matching entry point.
- `g:/Programming/Repos/teamy-mft/src/query/query_group.rs` — defines one AND-group within a query plan.
- `g:/Programming/Repos/teamy-mft/src/query/query_rule.rs` — defines the supported rule kinds for the new fast query engine.
- `g:/Programming/Repos/teamy-mft/src/query/query_needle.rs` — owns the case-insensitive matching primitives used by the fast rule engine.
- `g:/Programming/Repos/teamy-mft/src/search_index/load.rs` — exposes zero-copy row views over the current mapped segment-based index format and rejects stale index versions with a rebuild prompt instead of maintaining legacy parsing code.
- `g:/Programming/Repos/teamy-mft/src/search_index/format.rs` — current versioned disk format header; the data area is now segment-based rather than full-path row based.
- `g:/Programming/Repos/teamy-mft/src/search_index/search_index_bytes.rs` — current center of the segment dictionary, parent-chain node encoding, byte parsing, and snapshot regression coverage.
- `g:/Programming/Repos/teamy-mft/notes/search-index.md` — existing long-term indexing ideas; should be updated to reflect the new immediate focus on two-rule matching and zero-copy storage.
- `g:/Programming/Repos/teamy-mft/notes/parallel work.md` — still relevant for task decomposition and scheduling once the matcher is removed.
- `d:/Repos/Azure/Cloud-Terrastodon/crates/azure/src/resource_groups.rs` — request-object and `IntoFuture` style reference.
- `d:/Repos/Azure/Cloud-Terrastodon/crates/azure_devops/src/azure_devops_agent_pool_entitlements_for_project.rs` — parameterized request-object example to mirror at the API edge.
- `g:/Programming/Repos/icu4x/utils/zerotrie/Cargo.toml` — confirms available `zerotrie` crate version and features to plan against.
- `g:/Programming/Repos/icu4x/utils/zerotrie/examples/first_weekday_for_region.rs` — concrete example of compact immutable byte-backed trie data.

**Verification**
1. Run `cargo check` and `cargo check --features tracy` after each major refactor step.
2. Re-run `cargo run --release --features tracy -- query "'flower .jar$" --debug` or an equivalent representative suffix query adapted to the new parser, and compare Tracy captures before and after the rule-engine migration.
3. Add focused tests for parsing and evaluating `contains_case_insensitive` and `ends_with_case_insensitive`, including mixed-case inputs and deleted/non-deleted filtering interactions.
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

**Further Considerations**
1. `contains_case_insensitive` is usually harder to accelerate with trie structures than `ends_with_case_insensitive`, so the disk format may need a hybrid design: compact byte-backed storage for sequential scans plus trie-backed dictionaries for repeated string components.
2. The new rule engine now scans normalized path segments instead of full normalized paths, so the next storage optimization is more likely postings or segment-level indexes than additional full-path compaction.
3. If backwards compatibility of `.mft_search_index` matters, plan an explicit format version bump and migration story instead of trying to keep the old row layout partially alive.
4. The current Tracy captures show that parallel per-drive loading is no longer the main issue; the dominant remaining work is per-query search-index parsing and per-rule full segment scans within each drive, so the next material win is algorithmic candidate reduction rather than more coarse-grained parallelism.
