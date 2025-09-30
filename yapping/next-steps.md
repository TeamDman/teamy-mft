- [x] update resolve paths simple to take the drive letter
- [ ] update resolve paths simple to handle multiple parent attributes

# Next Steps: High-Speed MFT -> Canonical Path List Pipeline

## 1. Objective
Transform a dumped raw `$MFT` file into a stream (and/or collected list) of canonical NTFS paths as fast as possible using:
- Read-only mmap (input)
- Single bulk copy into `Vec<u8>` (mutable workspace)
- Parallel in-place fixup application
- Parallel attribute scanning (extract filenames / parent references only)
- Parent resolution to full canonical paths (supporting multiple hardlink parents)
- Streaming output (print as soon as a path becomes resolvable) while continuing to resolve the remainder

We add focused, minimal new APIs to the existing `mft` crate (zero-copy slice-based) without a giant rewrite now.

## 2. Key Performance Principles
1. Single pass copies & fixups (O(N) memory bandwidth bound).
2. Avoid per-entry allocation (store lightweight POD structs referencing the master `Vec<u8>` slice).
3. Use direct index math: `entry_offset = record_number * entry_size`.
4. SIMD opportunistically (micro-optimizations only after profiling):
   - Fixup tail compare (16B wide loads / unrolled loop)
   - UTF-16 validity / length scanning (optional)
5. Parallelism via Rayon (`par_chunks_exact(entry_size)`).
6. Parent resolution uses arrays, not hash maps, for O(1) lookups.
7. Hardlink (multiple FILE_NAME attributes) captured as multiple parent/name pairs per entry.

## 3. Assumptions & Constraints
- Dumped file is a raw, *unmodified* image of `$MFT` (fixups not yet applied).
- Entry size constant across file (derive from first entry header or provided by caller).
- Record reference -> index mapping: record number (low 48 bits usually) == entry index (validate).
- Canonical path rule: choose Win32 / Win32+DOS namespace over DOS or POSIX; if multiple hardlinks, produce all canonical paths (one per distinct parent chain).
- We can tolerate small buffering latency before a path prints (prefer early streaming when parent chain becomes available).

## 4. Data Structures
```
struct RawMft { data: Vec<u8>, entry_size: usize, entry_count: u64 }

// One filename instance (could be multiple per entry)
struct FileNameRef<'a> {
  entry_id: u32,          // or u64 if needed
  parent_id: u32,         // raw reference mapped to index
  namespace: u8,
  name_utf16: &'a [u16],  // slice into RawMft.data
}

// Compact staging after extraction
struct NamesIndex<'a> {
  per_entry: Vec<SmallVec<[usize;2]>>; // indices into file_names for each entry (hardlinks)
  file_names: Vec<FileNameRef<'a>>;
}

// Path resolution cache (lazy)
enum PathState {
  Unresolved,
  Resolving,            // cycle detection guard
  Resolved(PathBuf),
  Failed,               // corrupt chain
}
```

## 5. Processing Pipeline (Detailed)
### 5.1 mmap + Copy
- Use `memmap2` to map file read-only.
- Copy into `Vec<u8>` (`reserve_exact(len)` then `extend_from_slice`). Rationale: writable contiguous buffer & ability to free file handle early.

### 5.2 Detect & Record Entry Size
- Parse first entry header (already implemented logic) with a new function: `mft::entry::detect_entry_size(entry_bytes: &[u8]) -> Result<u32>`.
- Validate file length % entry_size == 0.

### 5.3 Parallel Fixup Application
Add function in `mft` crate (new module `fast_fixup`):
```
pub fn apply_fixups_parallel(buf: &mut [u8], entry_size: usize) -> FixupStats;
```
- Iterate in parallel over mutable chunks.
- For each entry:
  - Quick `needs_fixup(entry)` check (tail == update_sequence?).
  - Apply fixups (as current logic) without logging unless invalid (count invalid).
- Return stats: {applied, already_applied, invalid}.

### 5.4 Parallel Attribute Scan (Filename Extraction)
Add zero-copy helper in `mft`:
```
pub struct FastEntry<'a> { bytes: &'a [u8], id: u32 }
impl<'a> FastEntry<'a> {
  pub fn for_each_filename<F: FnMut(FileNameRef<'a>)>(&self, f: F);
}

pub fn par_collect_filenames(data: &[u8], entry_size: usize) -> NamesIndex;
```
Implementation details:
- Use `par_chunks_exact(entry_size)` enumerate.
- Skip non-FILE signature.
- Single linear attribute walk: stop if attribute type sentinel 0xFFFFFFFF.
- When seeing attribute type 0x30 (FILE_NAME): capture namespace, parent reference (8 bytes), name length/namespace at offsets 0x40/0x41 of value.
- Filter out namespace if undesired (optional). For canonical preference later: keep all but mark preference ordering.
- Push each `FileNameRef` into `file_names`. Save index in `per_entry[entry_id]` (using `SmallVec<[usize;2]>`: typical 1–2 hardlinks).

### 5.5 Parent Reference Normalization
- NTFS parent reference: high 48 bits = entry number, low 16 bits sequence (or vice versa depending format). Implement mask:
```
fn parent_record_number(refnum: u64) -> u32 { (refnum & 0xFFFFFFFFFFFF) as u32 }
```
- Validate < entry_count; else mark as orphan (skip or collect under special root).

### 5.6 Canonical Path Resolution Algorithm
Goal: produce paths quickly, streaming once parent resolved.
Approach:
1. Maintain `Vec<AtomicU8>` for state flags + `Vec<Mutex<Option<PathBuf>>>` or lock-free structure. For performance & simplicity: two-phase resolution may suffice.
2. Phase A (parallel find roots): entries whose parent == self or invalid -> base nodes.
3. Phase B iterative frontier expansion:
   - Use a work queue (`crossbeam::deque`) seeded with roots.
   - Worker pops entry, processes its children (from reverse index: need `Vec<Vec<u32>>` mapping parent->children). Build this reverse map during filename extraction (push child id). Typical memory: 4 bytes * links.
   - For each child, attempt to resolve all its parent link variants: produce each path variant by cloning parent path + component (UTF-16 -> UTF-8 decode). Emit to output sink (print) immediately.
   - If multiple parent refs: each yields separate path (hardlink). Canonical variant selection: choose namespace order [Win32, Win32AndDos, Posix, Dos]. Deduplicate identical final path strings optionally with a Bloom filter (optional future optimization).

Simpler initial approach (no streaming):
- Topological style single-thread DFS with memoization (Vec<PathState>) might be fast enough; profile. Then parallelize if necessary.

### 5.7 UTF-16 to UTF-8 Conversion
- On-demand during path assembly only.
- Use `widestring` or manual `decode_utf16` (std) to avoid allocation explosion.
- Cache decoded names? Probably no: typical file names reused rarely across entries.

### 5.8 Output
- Provide two APIs:
  1. `fn collect_paths(&self) -> Vec<PathBuf>` (simple, full materialization)
  2. `fn stream_paths<F: FnMut(&str)>(&self, f: F)` (invokes callback as resolved)

## 6. Additions to mft Crate (Incremental)
1. Module `fast_fixup.rs`:
   - `needs_fixup(entry: &[u8]) -> bool`
   - `apply_fixup_in_place(entry: &mut [u8])`
   - `apply_fixups_parallel(buf: &mut [u8], entry_size: usize) -> FixupStats`
2. Module `fast_entry.rs`:
   - `FastEntry` struct + `for_each_filename`.
   - `parse_first_entry_size(bytes: &[u8]) -> Result<u32>`.
3. Module `fast_filenames.rs`:
   - `FileNameRef` struct.
   - `par_collect_filenames(...) -> NamesIndex`.
4. Module `path_resolve.rs`:
   - `build_reverse_index(per_entry: &Vec<SmallVec<[usize;2]>>, file_names: &Vec<FileNameRef>) -> Vec<Vec<u32>>`
   - `resolve_all_paths(names: &NamesIndex, reverse: &Vec<Vec<u32>>, decode: impl Fn(&[u16]) -> Cow<str>) -> Vec<PathBuf>`.
   - (Optional) parallel streaming resolver.
5. Public facade `fast.rs` re-exporting the above.

## 7. Parallelism Plan
- Feature flag `rayon` gates parallel functions.
- If `rayon` absent: fall back to sequential loops (still zero-copy, minimal overhead).
- Path resolution parallelization (Phase 2) deferred until after baseline correctness & profiling.

## 8. SIMD Opportunities (Later, Feature `simd`)
- Use `std::arch::x86_64::_mm_cmpeq_epi16` for tail vs update_sequence comparisons across multiple entries batched (micro). May not be needed.
- UTF-16 ASCII-fast path: check if all code units <128 via 128-bit/256-bit vector; if so, direct widening to bytes.
- Gate behind `cfg(target_feature = "sse2")` / `avx2` + runtime dispatch if necessary.

## 9. Hardlinks Handling
- Store all filename attributes.
- For canonical path printing: produce each parent path + chosen namespace variant.
- Namespace priority constant array.
- Dedup logic (optional): maintain `FxHashSet` if memory acceptable. Initially skip for speed.

## 10. Incremental Implementation Phases
Phase 1 (Core Infrastructure): mmap + copy + detect size + parallel fixups (bench).
Phase 2 (Filename extraction) + tests comparing with existing parser for sample entries.
Phase 3 (Parent resolution simple sequential) + correctness (spot check random paths vs existing tool).
Phase 4 (Parallel optimization of fixups & extraction combined) + path streaming.
Phase 5 (Hardlink multi-parent full support & namespace prioritization).
Phase 6 (Optional SIMD) after profiling identifies hotspots.
Phase 7 (API polishing + documentation + benchmarks recorded).

## 11. Benchmarks & Metrics
Collect during each phase:
- Time to copy 4 GB file.
- Fixup throughput (GB/s)
- Filename extraction throughput (entries/s)
- Path resolution total time.
- Peak RSS.
- Compare vs current implementation baseline.

## 12. Validation & Testing
- Unit tests: fixup correctness (tails replaced match fixup array). Roundtrip detection on known sample.
- Property tests (optional) for synthetic entries (valid/invalid boundary cases).
- Cross-check: Random sample of entries parsed by old code vs new for filename + parent.
- Orphan detection: entries whose parent >= entry_count -> mark and ensure they do not panic.
- Cycle detection: artificially craft cycle -> ensure resolver marks Failed.

## 13. Logging & Telemetry
- Info: overall timings per pipeline phase.
- Debug (optional): counts of namespaces, hardlink multiplicities distribution.
- Warn: invalid fixup entries, corrupt attributes.

## 14. Memory Footprint Estimates (4 GB MFT @ 1 KB entries ≈ 4M entries)
- `per_entry` Vec<SmallVec<[usize;2]>>: 4M * (pointer + inline storage) ≈ ~64–96 MB worst-case (hardlinks rare; typical closer to ~32 MB or less).
- `file_names` each: ~ (24 bytes) * average 1.1 ≈ ~105 MB.
- Temporary PathVec (if materializing all): assume average path len 80 bytes -> 320 MB (worst). Streaming mode eliminates this.
- Acceptable within modern RAM; streaming recommended for large path sets.

## 15. Deferred / Optional
- Sidecar index file caching names/parents for repeated analyses.
- Path dedup filter for massive hardlink sets.
- Persisted mapping of record number -> resolved path for incremental scans (delta updates).

## 16. Immediate Action Items
1. Implement `fast_fixup.rs` + benchmark.
2. Implement `fast_entry.rs::parse_first_entry_size`.
3. Implement `par_collect_filenames` returning `NamesIndex`.
4. Simple sequential resolver producing vector (baseline correctness).
5. Integrate into existing `check` command behind `--fast` flag.
6. Add timing/log instrumentation.

---
This plan keeps changes localized, enables quick wins, and sets a path to further optimize only after observing profiler data.

## 17. Zero-Copy Struct Casting / Transmute Considerations
Goal: Further shave cycles off header & attribute header decoding by replacing manual little-endian field extraction with direct pointer casting.

### 17.1 Potential Approaches
- Unsafe `ptr::read_unaligned` into a `#[repr(C)]` (or `#[repr(C, packed)]`) Rust struct representing the NTFS FILE record header.
- Use crates providing safe(ish) zero-copy views:
  - `bytemuck` (derive `Pod`, `Zeroable`).
  - `zerocopy` (derive `FromBytes`, `Unaligned`).
  - `scroll` (for endian-aware reading with minimal overhead).
- Hand-written inline functions doing unaligned loads (current approach) already very close to optimal; savings may be < 3% of total pipeline.

### 17.2 Header Structure Constraints
NTFS FILE record header is not naturally aligned for all fields if we pack it; using `repr(C)` without `packed` may introduce padding differences vs on-disk layout. Safest is to define a packed struct and copy into a local aligned struct if needed.
```
#[repr(C, packed)]
pub struct FileRecordHeaderRaw {
    pub signature: [u8;4],       // "FILE"
    pub usa_offset: u16,         // +0x04
    pub usa_size: u16,           // +0x06
    pub lsn: u64,                // +0x08
    pub sequence: u16,           // +0x10
    pub hard_link_count: u16,    // +0x12
    pub first_attr_offset: u16,  // +0x14
    pub flags: u16,              // +0x16
    pub used_size: u32,          // +0x18
    pub allocated_size: u32,     // +0x1C
    pub base_file_ref: u64,      // +0x20
    pub next_attr_id: u16,       // +0x28
    // ... rest not always needed for filenames
}
```
Because it is `packed`, direct field access produces unaligned loads (compiler may emit bytewise copies). To avoid UB we must not create references to packed fields; instead `ptr::read_unaligned` / derive `zerocopy::FromBytes` & `Unaligned`.

### 17.3 Safety / UB Risks
- Misaligned access UB if we create `&u32` pointing into packed struct; must copy out.
- Endianness: All NTFS metadata fields are little-endian; host is also x86 little-endian, so direct copy is fine on current targets but still logically little-endian conversions should be explicit for portability.
- Version / layout drift: Future NTFS versions adding fields at tail will not break fixed offset layout for existing fields; still we must bounds-check slice length >= header size before casting.

### 17.4 Recommended Pattern
```
fn parse_header(entry: &[u8]) -> Option<FileRecordHeaderRaw> {
    if entry.len() < core::mem::size_of::<FileRecordHeaderRaw>() { return None; }
    // SAFETY: size checked; layout is plain bytes; allows unaligned read.
    let mut hdr = core::mem::MaybeUninit::<FileRecordHeaderRaw>::uninit();
    unsafe { std::ptr::copy_nonoverlapping(entry.as_ptr(), hdr.as_mut_ptr() as *mut u8, core::mem::size_of::<FileRecordHeaderRaw>()); }
    Some(unsafe { hdr.assume_init() })
}
```
Or with `bytemuck`:
```
bytemuck::try_from_bytes::<FileRecordHeaderRaw>(&entry[..size])
```
(provides validation that type is `Pod`).

### 17.5 Attribute Header Zero-Copy
Resident attribute header (first 24 bytes after type & length): define a small packed struct and copy same way. Avoid parsing attribute body unless type == 0x30.
```
#[repr(C, packed)]
struct ResidentAttrHeaderRaw { /* fields up to value_offset/value_length */ }
```

### 17.6 Expected Gains
- Manual parsing cost per entry currently: a dozen `from_le_bytes`; attributing maybe ~20–40 ns per parsed entry depending on branch prediction. For 4M entries: ~80–160 ms worst case.
- Zero-copy may remove ~30–40% of that small portion -> net absolute win maybe 30–60 ms (<5% of total pipeline including path resolution). Beneficial but not critical.

### 17.7 When to Implement
Phase after baseline is working & profiled (post Phase 3 or 4). Only implement if profiling shows header extraction among top hotspots (unlikely once path building dominates).

### 17.8 SIMD vs Struct Copy
- 32 or 48-byte memcpy is usually inlined & vectorized by compiler already. Custom SIMD loads unlikely to beat `ptr::copy_nonoverlapping`.

### 17.9 Fallback / Feature Flag
Add feature `zero-copy-header` gating these unsafe casts. Default remain manual parsing until stabilized. Provide A/B benchmark harness comparing `parse_header_manual` vs `parse_header_zerocopy`.

### 17.10 Testing Strategy
- Validate field equality for first N entries (e.g., 10k) between manual & zero-copy versions.
- Fuzz with truncated slices to ensure graceful `None`.
- Sanitizer (ASAN / Miri) runs with feature enabled.

### 17.11 Hardlink / Attribute Interaction
Zero-copy only used for header & attribute headers; FILE_NAME attribute body still needs manual extraction (variable length); no additional savings expected there beyond skipping uninterested types.

### 17.12 Decision Matrix
| Aspect | Manual Parsing | Zero-Copy Packed | Zero-Copy + bytemuck |
|--------|----------------|------------------|----------------------|
| Safety | Safe           | Unsafe footguns  | Safer (Pod check)    |
| Speed  | Good           | Slightly better  | Similar              |
| Portability | High      | Little-endian assumed | Little-endian assumed |
| Complexity | Low        | Medium           | Medium               |

Recommendation: Implement AFTER correctness & baseline optimization unless micro-bench shows significant bottleneck.
