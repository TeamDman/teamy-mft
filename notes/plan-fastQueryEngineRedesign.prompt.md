## Plan: Fast Query Engine Redesign

Replace the current Nucleo-based fuzzy query path with a purpose-built fast first-pass engine that supports only two operations: `contains_case_insensitive` and `ends_with_case_insensitive`. Pair that engine rewrite with a storage redesign toward zero-copy indexed data on disk, using `zerotrie` where it fits the string dictionary layer, so query time is dominated by compact scans and simple rule evaluation instead of fuzzy-matcher setup, path cloning, and full-path heap allocation.

**Progress Update**
1. Completed: the fast query path no longer uses Nucleo. Matching now uses the custom rule engine limited to `contains_case_insensitive` and `ends_with_case_insensitive`.
2. Completed: query parsing was moved into `src/query/` and split into dedicated types such as `QueryPlan`, `QueryGroup`, `QueryRule`, and `QueryNeedle`.
3. Completed: CLI query inputs now normalize both repeated positional arguments and `|`-separated segments into the same OR-of-ANDs query plan.
4. Completed: the sync pipeline was cleaned up to use `Vec<DriveSyncInfo>` worklists instead of `BTreeMap<char, ...>` in the main flow, reducing incidental complexity while the query work was landing.
5. Completed: baseline validation is green again; `./check-all.ps1` passes after the refactors.
6. Completed: the search-index loader now exposes borrowed `SearchIndexPathRowView` values from the memory map, and the query path allocates owned `IndexedPathRow` values only for matches.
7. In progress: the on-disk search-index format is still the original row-oriented layout, so zero-copy scanning is improved, but normalized storage and `zerotrie` integration have not landed yet.

**Steps**
1. Completed Phase 1: narrowed the product boundary in the query layer so the indexed fast path is explicitly a rough first pass supporting only `contains_case_insensitive` and `ends_with_case_insensitive`.
2. Completed Phase 1: defined the internal rule model in `src/query/`, with parsing and normalization logic that produces OR-of-ANDs query plans from both repeated positional inputs and `|` syntax.
3. Completed Phase 1: removed Nucleo from the execution design and replaced matcher setup, injection, tick loops, and snapshot reads with direct rule evaluation over indexed records.
4. Partially completed Phase 1: adopted the request-object style structurally by splitting query logic into dedicated types and modules, but the explicit `IntoFuture` request-object orchestration has not been introduced yet.
5. Partially completed Phase 2: improved zero-copy reads by iterating borrowed row views from the memory map in `src/search_index/load.rs`, while keeping the old owned `rows()` path as a compatibility wrapper.
6. Remaining Phase 2: redesign the on-disk `.mft_search_index` format so metadata, offsets, normalization, and string storage are arranged intentionally for zero-copy query evaluation rather than the original row-oriented serialization.
7. Remaining Phase 2: introduce `zerotrie` where it materially improves the immutable string representation, likely as part of normalized string dictionaries or segment tables rather than as a blanket replacement for all query logic.
8. Remaining Phase 2: decide the exact indexed granularity and normalization strategy for the new format so `contains_case_insensitive` and `ends_with_case_insensitive` can operate without repeated lowercase conversions or unnecessary string materialization.
9. Remaining Phase 3: rework the execution pipeline around smaller owned request stages or task stages so load, decode, filter, and materialization are more explicitly modeled and instrumented.
10. Remaining Phase 3: add instrumentation that distinguishes index open time, row-view iteration time, rule evaluation time, candidate materialization time, and output time so the next Tracy captures show where the remaining latency lives.
11. Remaining Phase 4: verify behavior and latency against representative contains and suffix queries, compare with the previous implementation, and remove any leftover compatibility paths once the new storage format is in place.

**Relevant files**
- `g:/Programming/Repos/teamy-mft/src/cli/command/query/query_cli.rs` — current indexed query entry point; now consumes `QueryPlan` and borrowed row views, and materializes owned rows only for matches.
- `g:/Programming/Repos/teamy-mft/src/query/query_plan.rs` — owns the OR-of-ANDs query normalization and is the current center of the parser semantics.
- `g:/Programming/Repos/teamy-mft/src/query/query_group.rs` — defines one AND-group within a query plan.
- `g:/Programming/Repos/teamy-mft/src/query/query_rule.rs` — defines the supported rule kinds for the new fast query engine.
- `g:/Programming/Repos/teamy-mft/src/query/query_needle.rs` — owns the case-insensitive matching primitives used by the fast rule engine.
- `g:/Programming/Repos/teamy-mft/src/search_index/load.rs` — now exposes zero-copy row views over the mapped index and still provides the old owned-row compatibility path.
- `g:/Programming/Repos/teamy-mft/src/search_index/format.rs` — current row-oriented disk format; main redesign target for zero-copy representation and normalized storage.
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

**Decisions**
- Nucleo is out of scope for the new fast path.
- The supported operations are intentionally limited to `contains_case_insensitive` and `ends_with_case_insensitive`.
- The fast query path is a rough first pass, not a full advanced filtering engine.
- `zerotrie` is part of the storage redesign plan, especially for compact immutable string representation.
- The request-object plus `IntoFuture` style from Cloud-Terrastodon remains the orchestration pattern.
- Zero-copy and normalized storage are now first-class performance goals alongside CPU saturation.

**Further Considerations**
1. `contains_case_insensitive` is usually harder to accelerate with trie structures than `ends_with_case_insensitive`, so the disk format may need a hybrid design: compact byte-backed storage for sequential scans plus trie-backed dictionaries for repeated string components.
2. If the new rule engine still spends most time scanning full normalized paths, the next storage optimization may be specialized suffix tables or segment-level indexes rather than more scheduler work.
3. If backwards compatibility of `.mft_search_index` matters, plan an explicit format version bump and migration story instead of trying to keep the old row layout partially alive.
