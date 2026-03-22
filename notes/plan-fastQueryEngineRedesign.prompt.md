## Plan: Fast Query Engine Redesign

Replace the current Nucleo-based fuzzy query path with a purpose-built fast first-pass engine that supports only two operations: `contains_case_insensitive` and `ends_with_case_insensitive`. Pair that engine rewrite with a storage redesign toward zero-copy indexed data on disk, using `zerotrie` where it fits the string dictionary layer, so query time is dominated by compact scans and simple rule evaluation instead of fuzzy-matcher setup, path cloning, and full-path heap allocation.

**Steps**
1. Phase 1: Narrow the product boundary in the query layer. Update the implementation targets in `g:/Programming/Repos/teamy-mft/src/cli/command/query/query_cli.rs` so the indexed fast path is explicitly defined as a rough first pass supporting only `contains_case_insensitive` and `ends_with_case_insensitive`. Advanced filtering is deliberately pushed downstream.
2. Phase 1: Define a small internal rule model for the new engine, for example `QueryRule::ContainsCaseInsensitive` and `QueryRule::EndsWithCaseInsensitive`, plus parsing logic that converts the current user query string into one or more of those rules. Keep the parser deliberately conservative and reject or degrade unsupported fuzzy semantics rather than reintroducing Nucleo-like complexity.
3. Phase 1: Remove Nucleo from the execution design. Replace matcher setup, injection, tick loops, and snapshot reads with a direct evaluator that scans indexed records and applies the two supported rules in a predictable order. This should happen in the existing query path first, even before the disk-format redesign is complete, so behavior and performance can be measured independently of storage changes.
4. Phase 1: Retain the request-object plus `IntoFuture` architectural style from the Cloud-Terrastodon examples, but retarget the stage design around the custom rule engine instead of a matcher pipeline. The top-level `IndexedQueryRequest` should orchestrate load, decode or map, evaluate rules, collect candidate rows, and print results.
5. Phase 2: Redesign the on-disk `.mft_search_index` format for zero-copy reads. The new format should separate metadata, offsets, and string storage so the query path can evaluate rules against borrowed bytes or compact slices rather than eagerly allocating `String` for every path.
6. Phase 2: Introduce `zerotrie` into the storage plan where it provides real value: as a compact immutable dictionary or prefix-oriented lookup structure for normalized path strings or path segments. The goal is not to force every query through trie traversal, but to use `zerotrie` to improve disk representation and reduce parse overhead for repeated string material.
7. Phase 2: Decide the granularity of indexed strings. For the current constrained rule engine, prioritize what makes `contains_case_insensitive` and `ends_with_case_insensitive` fast on real workloads. That may mean storing normalized full paths plus compact offsets first, with segment dictionaries and richer tries as a later extension if measurements justify them.
8. Phase 2: Design normalization into the format. Because both supported rules are case-insensitive, store or derive a normalized representation once during indexing so query-time matching does not repeatedly lowercase or allocate temporary strings. Make this normalization strategy explicit in the file format and query rules.
9. Phase 3: Rework the execution pipeline around smaller owned request stages. Keep the Cloud-Terrastodon-inspired request structs and Tokio scheduling model, but have stages produce batches of lightweight row views or offsets instead of heavyweight owned path objects with cloned strings. Blocking decode or mapping work should stay inside `tokio::task::spawn_blocking` when necessary.
10. Phase 3: Add instrumentation that reflects the new design: index open time, zero-copy decode or slice setup time, rule evaluation time for `contains_case_insensitive`, rule evaluation time for `ends_with_case_insensitive`, candidate materialization time, and output time. Tracy should make it obvious whether storage, scanning, or printing is the dominant remaining cost.
11. Phase 4: Verification and migration. Validate that the new engine returns the same rough-pass results expected for representative contains and suffix queries, compare total latency against the current Nucleo-based path, and only then remove or quarantine old fuzzy-specific code paths and tests.

**Relevant files**
- `g:/Programming/Repos/teamy-mft/src/cli/command/query/query_cli.rs` — current indexed query entry point; primary place where Nucleo orchestration is replaced by the custom rule engine request flow.
- `g:/Programming/Repos/teamy-mft/src/search_index/load.rs` — current eager row decoder; likely to be replaced or split into zero-copy mapping and lightweight row-view readers.
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
