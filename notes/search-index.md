# Search Index Implementation Plan

## Goals

1. Add sync sub-modes:
	- `teamy-mft sync mft`
	- `teamy-mft sync index`
	- `teamy-mft sync` (default: run `mft` then `index`)
2. Build a path-search-focused index representation from `.mft` data.
3. Minimize `teamy-mft query` latency by avoiding full-path fuzzy matching over all rows.
4. Support hierarchical constraints like:
	- parent/ancestor matches `X`
	- descendant/file-name matches `Y`
5. Keep index files adjacent to MFT snapshots:
	- `C.mft`
	- `C.mft_search_index`
6. Continue using `nucleo` for fuzzy matching, but run it against deduplicated path segment names (not full paths).
7. Use `zerotrie` for immutable synced snapshot data; reserve `masstree` for future live change overlays (USN/watch mode).

---

## Non-goals (for first iteration)

- No semantic/vector indexing in this phase.
- No live USN tailing in this phase (batch snapshot only).
- No cross-machine stable identity solution in this phase.

---

## Current Baseline (what we are replacing)

- `query` loads `.mft`, reconstructs full paths, then pushes full path string into nucleo.
- Matching is done on one full path column, so every query pays path reconstruction + fuzzy work.
- `sync` only produces `.mft` files.

Latency bottlenecks today:
- cold disk read + parsing
- full path string allocation
- fuzzy matching across a very large string corpus

---

## Target Architecture

### 1) Two-stage sync pipeline

Per selected drive:

1. **MFT stage**
	- Read physical MFT bytes.
	- Persist `<drive>.mft`.
	- Keep bytes in memory for downstream stage in same process invocation.

2. **Index stage**
	- Consume in-memory bytes when available (no reread).
	- Otherwise (for `sync index`) read `<drive>.mft` from disk.
	- Build and persist `<drive>.mft_search_index`.

### 2) Query over index, not raw reconstructed paths

- Open `<drive>.mft_search_index`.
- Parse query into path-segment-oriented constraints.
- Run `nucleo` against unique segment names to get candidate `name_id`s.
- Execute constraint-first filtering using compact IDs/relations.
- Reconstruct full paths only for final matches to print.

### 3) Segment reuse as first-class model

Path segments are shared entities.

Examples:
- `a/hello/b.txt`
- `c/hello/d.txt`

`hello` is one segment string reused by multiple parent contexts and multiple child contexts.

Implication:
- model segment *string* separately from segment *node occurrence*.
- fuzzy match operates on segment strings.
- hierarchy constraints operate on node occurrences and parent links.

### 4) Snapshot + overlay design

- **Snapshot layer (infrequent sync):** immutable `zerotrie`-backed segment dictionary + compact arrays in `.mft_search_index`.
- **Overlay layer (future watch mode):** mutable `masstree` for recent renames/adds/deletes from USN.
- Query reads both, with overlay taking precedence for conflicts.

---

## CLI Changes

## Desired UX

- `teamy-mft sync`
  - Equivalent to `teamy-mft sync mft` then `teamy-mft sync index`
- `teamy-mft sync mft`
  - Only refresh `.mft` snapshots.
- `teamy-mft sync index`
  - Build indexes from existing `.mft` snapshots.

### Clap shape proposal

Replace `SyncArgs`-only command with a nested sync command model:

- `Sync(SyncCommand)` where `SyncCommand` contains:
  - `mode: Option<SyncModeSubcommand>`
  - shared options (`drive_pattern`, `overwrite_existing`)

`SyncModeSubcommand`:
- `Mft`
- `Index`

Behavior:
- `None` => run both stages.
- `Mft` => only MFT stage.
- `Index` => only index stage.

Implementation note:
- Keep option parity so existing scripts continue to work with `sync` default.

---

## On-disk Index Format (`.mft_search_index`)

Use a custom binary v1 designed for fast mmap/stream decode and minimal allocations.

## Header

- magic bytes: `TMFTIDX\0`
- version: `u16` (start at 1)
- flags: `u16`
- drive letter: `u8`
- created_at_unix_ms: `u64`
- record counts (u32/u64 as appropriate)
- offsets to sections

## Sections

1. **Name pool** (unique path segment strings)
	- deduplicated segment strings (case-folded canonical form)
	- serialized as `zerotrie` payload for compact immutable lookup
	- ID space: `name_id: u32`

2. **Node table** (MFT-derived hierarchy graph)
	- one row per filesystem node (file or directory):
	  - `node_id: u32`
	  - `name_id: u32`
	  - `parent_node_id: u32` (root/self sentinel allowed)
	  - `flags: u16` (is_dir, deleted, etc.)
	  - optional `mft_ref: u64`

3. **Name postings**
	- `name_id -> sorted [node_id...]`
	- supports exact segment hit filtering quickly

4. **Name-to-string bridge for nucleo**
	- compact contiguous view of segment strings by `name_id`
	- allows running `nucleo` on unique names only

5. **Optional fuzzy acceleration table (v1.1 or v2)**
	- lightweight n-gram index for segment text to candidate `name_id`s.
	- initial v1 can skip this and scan unique names if pool is manageable.

6. **Optional path materialization cache**
	- not full paths for all nodes by default.
	- allow deferred reconstruction using parent links.

Format principles:
- stable endianness (little-endian)
- contiguous arrays where possible
- mmap-friendly sections with pointer-free offsets
- sorted postings for cheap intersections
- checksum footer for corruption detection

---

## Query Semantics (segment + hierarchy aware)

### Query model

Split user query into segment tokens and operators.

Initial minimal syntax:
- whitespace-separated terms = AND
- term containing path separators (`/` or `\\`) is treated as ordered segment chain
- suffix regex-like intent `.jar$` becomes `ends_with(.jar)` matcher on segment name

Examples:
- `flower .jar$`
  - descendant segment fuzzy/exact contains `flower`
  - final segment ends with `.jar`
- `src/hello.java`
  - an ancestor segment matches `src`
  - descendant/file segment matches `hello.java`

### Execution strategy

1. Resolve segment terms to candidate `name_id` sets.
	- exact/suffix constraints by direct scans/lookups
	- fuzzy constraints by running `nucleo` on unique segment strings
2. Expand to candidate `node_id` sets via postings.
3. Apply structural filters:
	- ancestor/descendant checks via parent chain walk
4. Intersect candidate sets aggressively before expensive checks.
5. Reconstruct and print top `limit` final paths.

Why this reduces latency:
- fuzzy work scales with unique segment count, not full path count
- match operations happen on unique segment dictionary + integer IDs
- hierarchy checks run on compact node table, not full strings
- full path allocation only for output rows

---

## Detailed Implementation Steps

## Phase 1: CLI refactor for sync modes

1. Introduce sync subcommand enum (`mft`, `index`).
2. Keep current options (`drive_pattern`, `overwrite_existing`) available.
3. Implement default `sync` as sequential two-stage run.
4. Update help text, `ToArgs`, and docs.

Deliverable:
- `teamy-mft sync`, `teamy-mft sync mft`, `teamy-mft sync index` all wired.

## Phase 2: Extract reusable MFT snapshot pipeline

1. Refactor current sync internals into reusable functions:
	- `sync_mft_for_drives(...) -> Vec<DriveMftSnapshot>`
2. `DriveMftSnapshot` should include:
	- drive letter
	- output path
	- in-memory bytes (or shared buffer handle)
3. Preserve current threading model and privilege behavior.

Deliverable:
- MFT stage callable independently and composable with index stage.

## Phase 3: Index builder v1

1. Add module, e.g. `src/search_index/`:
	- `format.rs` (struct layout + serde)
	- `build.rs` (build from parsed MFT entries)
	- `load.rs` (read/mmap index)
	- `query.rs` (ID-level filtering primitives)
2. Build segment dictionary + node table + postings.
3. Serialize segment dictionary as `zerotrie` bytes for immutable lookup.
4. Serialize to `<drive>.mft_search_index.tmp` then atomic rename.
5. Add metadata/version validation.

Deliverable:
- Index files are generated from `.mft` and loadable.

## Phase 4: Hook `sync index` and default `sync`

1. `sync index`:
	- discover `<drive>.mft`
	- build `<drive>.mft_search_index`
2. default `sync`:
	- run MFT stage
	- pass in-memory snapshots directly to index builder
	- avoid rereading newly-written `.mft`

Deliverable:
- End-to-end two-stage sync with no duplicate MFT read in combined mode.

## Phase 5: Query command migration

1. Add indexed query path behind a flag initially:
	- `--use-search-index` (default off for one release)
2. Implement parser for segment/hierarchy constraints.
3. Route fuzzy segment matching through `nucleo` on unique segment dictionary.
4. Execute candidate filtering on index structures.
5. Reconstruct full paths only for survivors.
5. Compare output parity against old query mode.
6. Flip default to indexed path once stable.

Deliverable:
- Lower-latency query path with functional equivalence for common queries.

## Phase 6: Optimization + hardening

1. Keep hot query sections directly mmap-able (no per-query decompress).
2. Add optional zstd for cold/archive artifacts only (or background recompress), not hot query path.
3. Add binary compatibility tests and corruption tests.
4. Add benchmark harness for query p50/p95.

## Phase 7: Live-change overlay (later)

1. Add USN/watch ingestion path.
2. Store mutable delta index in `masstree` keyed by node identity/path keys.
3. Merge overlay + immutable snapshot at query time.
4. Periodically compact overlay into next snapshot (`sync index`).

Deliverable:
- fast mutable updates without rewriting full immutable snapshot.

Deliverable:
- Measured latency wins and robust format evolution story.

---

## Data Structures (v1 recommendation)

Use simple, fast primitives first:

- `Vec<NodeRecord>` node table
- `zerotrie` byte payload for immutable name pool with `name_id`
- `Vec<PostingRange>` + flat `Vec<u32>` postings payload

`NodeRecord` (conceptual):
- `name_id: u32`
- `parent_node_id: u32`
- `flags: u16`
- `reserved: u16`

Conceptual identity split:
- `name_id`: unique segment string identity (e.g., `hello`)
- `node_id`: occurrence identity (a specific file/dir entry in a specific parent chain)

This split is required for cases where one segment string has many parents and many children.

Adopt `zerotrie` in v1 for immutable snapshot dictionary.
Reserve `masstree` for mutable watch-mode deltas in later phases.

---

## Matching Engine Plan

### Tokenization

- Normalize path separators (`/` and `\\` equivalent).
- Split chain expressions into ordered segment constraints.
- Distinguish:
  - exact segment
  - contains/fuzzy segment
  - suffix segment (e.g., `.jar$`)

### Candidate generation

- Exact segment => direct `name_id` lookup
- Fuzzy segment => run `nucleo` against unique segment dictionary, not full paths
- `name_id` -> candidate `node_id`s via postings

### Nucleo integration details

- Build one in-memory `nucleo` corpus per query from unique segment names in loaded index.
- Store `name_id` in matched item payload.
- Convert matched `name_id` set to candidate nodes via postings.
- Keep current smart-case/smart-normalization behavior for user continuity.

Complexity impact:
- old: fuzzy against all reconstructed paths
- new: fuzzy against unique segment names only, then integer expansion

### Hierarchy enforcement

- For each candidate node, walk `parent_node_id` chain until root.
- Confirm required ancestor sequence constraints.
- Short-circuit early on mismatch.

### Output

- Reconstruct path for final nodes only.
- Keep current deleted-entry highlighting behavior where possible.

---

## File Layout and Naming

Per drive in sync dir:

- `C.mft`
- `C.mft_search_index`
- optional temporary:
  - `C.mft_search_index.tmp`

Compatibility:
- If index missing, `query` can fall back to current raw `.mft` path.
- If index version mismatched, rebuild suggestion/error.

---

## Zero-copy + mmap notes

- Query path should mmap `.mft_search_index` and read sections by offset.
- Avoid deserializing into pointer-rich structures in hot path.
- Keep section layouts contiguous and alignment-safe.
- For `zerotrie`, map/copy section bytes into the expected buffer view once per load.
- Do not require zstd decompression for hot-path queries.

---

## Observability and Metrics

Add timings to logs:

- sync mft:
  - read ms
  - write ms
- sync index:
  - parse ms
  - build ms
  - serialize ms
- query:
  - load index ms
  - tokenization ms
  - candidate generation ms
  - hierarchy filter ms
  - path reconstruction ms
  - total ms

Track counts:
- total nodes
- unique segment names
- postings sizes
- candidates pre/post hierarchy filter

---

## Testing Strategy

1. **Unit tests**
	- format encode/decode round-trip
	- tokenization and constraint parsing
	- ancestor/descendant matcher correctness

2. **Integration tests**
	- build index from fixture `.mft`
	- query parity old vs new for curated cases
	- `sync` mode behaviors:
	  - default runs both
	  - `mft` only
	  - `index` only

3. **Benchmark tests**
	- fixed corpus, compare old query vs indexed query:
	  - cold start
	  - warm cache
	  - p50/p95 latency

---

## Rollout Plan

1. Land `sync` mode refactor + index format writer/reader.
2. Land indexed query behind opt-in flag.
3. Validate correctness + latency wins in real corpus.
4. Switch default query path to index.
5. Keep fallback mode for one release cycle.

---

## Risks and Mitigations

1. **Format churn risk**
	- Mitigation: versioned header + explicit migration/rebuild logic.

2. **Memory pressure on very large corpora**
	- Mitigation: streaming build, mmap read, avoid full path materialization.

3. **Query semantics regressions**
	- Mitigation: parity test suite + staged flag rollout.

4. **Over-optimization too early (trie crate lock-in)**
	- Mitigation: start with simple dictionary/postings v1; abstract name lookup behind trait.

---

## Suggested Immediate Next PRs

1. PR1: `sync` subcommand modes + shared stage orchestration skeleton.
2. PR2: `.mft_search_index` format v1 + builder + loader.
3. PR3: indexed query engine + `--use-search-index` + parity tests.
4. PR4: latency benchmarks + default switch.

This sequence gets usable wins quickly while preserving flexibility for deeper trie/zstd experiments later.
