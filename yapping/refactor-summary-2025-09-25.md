## Refactor summary (diff: 4c34e45..297d7d3)

This refactor restructures the logical-to-physical read pipeline, separates concerns between planning, I/O scheduling, and result assembly, and tightens correctness around sector alignment and ordering. It also standardizes sizing units to usize-based uom types. Below is a concise analysis grounded in the code changes in this diff.

## What changed at a glance

- New modules added under `src/read/`:
	- `active_physical_read_request.rs` — a repr(C) carrier for OVERLAPPED I/O, encapsulating a single async read request buffer and metadata.
	- `physical_reader.rs` — an IOCP-based reader that manages in-flight operations, queue saturation, and result collection.
	- `data_range_set.rs` — thin wrapper over `range_set::RangeSet`; a foundation for tracking written byte ranges (currently minimal, likely for future sparse validation/zeroing checks).
- Major rewrites:
	- `physical_read_request.rs` — type simplified and semantically clarified; ordering fixed.
	- `physical_read_plan.rs` — from ad-hoc vector + manual IOCP to BTreeSet-backed planning and a dedicated reader.
	- `logical_read_plan.rs` — redesigned to a clear logical file description with sparse vs physical segments.
	- `physical_read_results.rs` — result container now keyed/sorted by request; writer now stitches using the logical plan instead of “embedded logical offsets”.
- Unit type update: `uom::si::u64::Information` → `uom::si::usize::Information` across affected modules.
- Tests consolidated into module tests near implementations; legacy top-level tests for the old API were removed.

## Core data model changes

### PhysicalReadRequest

File: `src/read/physical_read_request.rs`

- Before: carried `physical_offset`, `logical_offset`, `length` (all u64::Information); `Ord` on physical offset only.
- After:
	- Fields: `offset: Information`, `length: Information` (usize-based).
	- Logical placement is no longer embedded here; it belongs to the logical plan.
	- New API: `align_to_sector_size(&mut self, sector_size)` expands [start,end) to sector boundaries.
	- Ordering: `impl Ord` compares `(offset, length)` ensuring Eq and Ord consistency for `BTreeSet` semantics.
	- Convenience: `physical_end() -> Information`.

Impact: Requests are purely “where and how much to read physically,” untangling them from logical layout. This simplifies de-duplication, merging, and set operations.

### LogicalReadPlan + LogicalFileSegment

File: `src/read/logical_read_plan.rs`

- Before: `LogicalReadSegment` with a `LogicalReadSegmentKind::{Physical{ physical_offset_bytes }, Sparse }`, stored in a Vec, plus `total_logical_size_bytes`.
- After:
	- `LogicalReadPlan { segments: BTreeSet<LogicalFileSegment> }` ordered by `logical_offset`.
	- `LogicalFileSegment { logical_offset, length, kind: LogicalFileSegmentKind::{ Physical { physical_offset }, Sparse } }`.
	- `as_physical_read_plan()` replaced with `as_physical_read_request()` on segment and `as_physical_read_plan()` on plan via `FromIterator` semantics to produce a `PhysicalReadPlan`.
	- `total_logical_size()` computed from the last segment: `last.logical_offset + last.length`, or zero.

Impact: Logical description is now a sorted set of logical ranges mapped to physical offsets or marked sparse. This is the authoritative source for output layout; no more “logical offsets smuggled” inside physical requests.

### PhysicalReadPlan

File: `src/read/physical_read_plan.rs`

- Before: `Vec<PhysicalReadRequest>` plus ad-hoc IOCP read implementation, logical offsets embedded, total requested bookkeeping.
- After:
	- Data: `BTreeSet<PhysicalReadRequest>` with deterministic ordering; `IntoIterator` and `FromIterator` implemented.
	- Behavior:
		- `push(PhysicalReadRequest)` with configurable zero-length behavior via `ZeroLengthPushBehaviour::{ Panic, NoOp }` (default Panic).
		- `merge_contiguous_reads()` merges adjacent requests when `last.physical_end() == next.offset`.
		- `chunked(chunk_size)` splits requests without altering total size.
		- `align_512()` expands requests to 512-byte sectors via request-local alignment, then re-merges.
		- `read(filename)` delegates to `PhysicalReader` with `MAX_IN_FLIGHT_IO = 32`.
	- Removed: embedded logical offsets and the in-function IOCP loop; these live in `PhysicalReader` now.

Impact: The plan is a pure collection with transformation utilities (merge/chunk/align). Reading is delegated, improving testability and separation of concerns.

### PhysicalReader + ActivePhysicalReadRequest

Files: `src/read/physical_reader.rs`, `src/read/active_physical_read_request.rs`

- PhysicalReader:
	- Owns: file handle, IOCP handle, remaining queue, in-flight count, result slots.
	- API: `try_new(filename, requests, max_in_flight)`, `enqueue_until_saturation()`, `try_enqueue()`, `receive_result()`, `read_all() -> PhysicalReadResults`.
	- Schedules up to `MAX_IN_FLIGHT_IO` concurrent reads, collects results in order, returns a `BTreeSet<PhysicalReadResultEntry>`.
- ActivePhysicalReadRequest:
	- repr(C) struct with OVERLAPPED as the first field, buffer Vec, response index, and original `PhysicalReadRequest`.
	- `new(request, response_index)` initializes OVERLAPPED with 64-bit file offsets.
	- `send(file_handle)` queues the read via `ReadFile` and leaks the Box so the allocation survives until IOCP completion.
	- `receive(completion_port)` calls `GetQueuedCompletionStatus`, reconstructs the Box from `lpOverlapped` pointer (relies on overlapped being the first field), truncates buffer to bytes_transferred, and returns `(PhysicalReadResultEntry, response_index)`.
	- Includes an invariant test ensuring the overlapped field is at offset 0.

Impact: Safe(ish) encapsulation of Windows overlapped I/O invariants; isolates unsafe FFI, lifetimes, and pointer arithmetic. The reader becomes a straightforward scheduler.

### PhysicalReadResults

File: `src/read/physical_read_results.rs`

- Before: `Vec<PhysicalReadResultEntry>` with `total_size`; writer used each entry’s logical_offset to seek and write, trimming alignment by simple deltas.
- After:
	- Data: `BTreeSet<PhysicalReadResultEntry>` (ordered by their `PhysicalReadRequest`), no cached `total_size`.
	- Writer: `write_to_file(&self, logical_plan: &LogicalReadPlan, output_path)`:
		- Pre-sizes the file to `logical_plan.total_logical_size()`.
		- Iterates logical segments; for physical segments it stitches together data potentially spanning multiple aligned reads.
		- Looks up the correct `PhysicalReadResultEntry` via `lower_bound` on the BTreeSet; if `lower_bound` overshoots, it steps back to the predecessor and validates containment.
		- Computes slice indices using `offset_within_entry` and writes into the correct logical position.
	- Tests:
		- Validates gap-zeroing by pre-sizing and not writing sparse regions.
		- Covers the “predecessor when aligned over-read” case (sector alignment earlier than requested offset).

Impact: The writer now derives logical placement from the LogicalReadPlan (single source of truth) and robustly resolves over-aligned physical reads. This fixes the classic “can’t find data for aligned-early read” bug.

### Units: u64 → usize

Affected files include `read` modules and `robocopy` parsing. The switch to `uom::si::usize::Information` reflects idiomatic sizing on 64-bit targets while keeping type-safety from uom. Parsing in `robocopy_log_parser.rs` now returns `Information` directly and composes with the rest of the pipeline without casting.

## Behavioral changes and correctness fixes

- Sector alignment is explicit: `PhysicalReadRequest::align_to_sector_size` expands read extents to sector boundaries; trimming occurs at write time by slicing.
- Predecessor-aware lookup in results: `write_to_file` uses BTreeSet `lower_bound` with predecessor fallback to handle over-reads starting before requested physical offsets.
- Deterministic ordering: `Ord` for `PhysicalReadRequest` now considers `(offset, length)` which fixes Eq/Ord consistency for sets and avoids accidental dedup of different extents sharing a start.
- Zero-length push policy: configurable via `ZeroLengthPushBehaviour` and defaulting to `Panic` to catch silent no-ops early.
- Separation of concerns: planning vs IO scheduling vs assembly/writing are separate modules with narrow APIs.

## The “end-state” mental model

Think in three layers:

1) Logical description (what the file looks like to consumers)
	 - `LogicalReadPlan { segments: BTreeSet<LogicalFileSegment> }`
	 - Each segment has `logical_offset`, `length`, and is either `Physical { physical_offset }` or `Sparse`.
	 - `total_logical_size()` defines the final file length.

2) Physical read plan (what to ask the device to read)
	 - `PhysicalReadPlan` as a set of `PhysicalReadRequest { offset, length }`.
	 - Transformations: merge contiguous, chunk into size-limited pieces, and align to sector size.
	 - No logical knowledge is embedded here.

3) Execution and assembly
	 - `PhysicalReader` performs overlapped I/O, returns `PhysicalReadResults` containing data keyed by request.
	 - `PhysicalReadResults::write_to_file(&logical_plan, path)` stitches the physical data into a logically correct output file, pre-sizing to cover sparse gaps.

Contract (inputs/outputs):
- Input: Logical plan (including sparse), device path.
- Internal: Derived physical plan → IOCP reads → ordered results.
- Output: A file whose length equals `logical_plan.total_logical_size()`, with bytes filled for physical segments and zeros where sparse.

Edge cases covered:
- Reads that start/end mid-sector (alignment handled by expansion + trim on write).
- Logical segments spanning multiple physical reads or crossing chunk boundaries (stitching loop advances within segment until done).
- Gaps between segments (file pre-sized; unwritten regions remain zeroed).
- Duplicate or overlapping physical requests (BTreeSet ordering and merge utilities aid dedup/merge).

## Why this transformation and why it’s better

- Single source of truth for logical layout: Logical placement is defined solely by `LogicalReadPlan`, eliminating duplicated or drifting logical offsets within physical requests.
- Safer and more testable I/O: Extracting IOCP handling into `PhysicalReader` and `ActivePhysicalReadRequest` encapsulates unsafe Windows FFI patterns (repr(C), pointer lifetimes, overlapped offsets) with clear invariants and unit tests.
- Deterministic planning: Using `BTreeSet` with correct `Ord/Eq` ensures consistent request ordering and proper set semantics.
- Robust alignment handling: The predecessor-aware lookup in writer fixes a subtle but frequent alignment failure mode.
- Clear extensibility: DataRangeSet and the modular design enable future enhancements (e.g., validating complete coverage, progress tracking, back-pressure tuning, sector size parametrization).
- Cleaner API surface: Each layer has a focused responsibility and smaller, composable APIs.

## How to spot opportunities to re-apply these lessons

Patterns to look for:

- Mixed logical/physical concerns: If a struct is carrying both logical placement and physical addressing, separate them. Keep logical layout authoritative and map to physical only where needed.
- Overlapped/async I/O with completion callbacks: Encapsulate OS-specific invariants (repr(C), field ordering, lifetime) in a dedicated type and centralize queueing/dequeueing logic.
- Alignment-sensitive I/O: Normalize with “expand to alignment” at the read stage and “trim to intent” at the write/assembly stage; add predecessor-aware lookups when using ordered sets.
- Ambiguous set semantics: If you store requests in ordered sets, ensure `Ord` and `Eq` agree on identity (use tuples of relevant fields). Add tests for dedup vs distinction.
- Edge-case policy toggles: Make zero-length or no-op scenarios explicit via a configurable policy (default to strict/Panic) to surface latent bugs.

Concrete re-application cues:
- Any code that had to “remember logical offsets” inside physical operations can be modernized to the plan/execute/assemble split used here.
- Any place where you use lower_bound on a set of ranges should consider predecessor checks to handle open intervals and alignment expansions.
- If you’re building concurrent/async file readers, factor out a per-operation carrier (like `ActivePhysicalReadRequest`) that owns buffer and metadata and document the invariants.

## Key APIs now (quick reference)

- Logical description:
	- `LogicalFileSegment { logical_offset, length, kind: Physical { physical_offset } | Sparse }`
	- `LogicalReadPlan::{ physical_segments(), as_physical_read_plan(), total_logical_size() }`

- Physical plan:
	- `PhysicalReadRequest::{ new(offset, length), align_to_sector_size(), physical_end() }`
	- `PhysicalReadPlan::{ push(), set_zero_length_behavior(), merge_contiguous_reads(), chunked(), align_512(), read() }`

- Execution + results:
	- `PhysicalReader::{ try_new(), enqueue_until_saturation(), try_enqueue(), receive_result(), read_all() }`
	- `PhysicalReadResults::{ new(), write_to_file(&LogicalReadPlan, path) }`

## Notes on related changes

- Robocopy parsing (`src/robocopy/robocopy_log_parser.rs`, `robocopy_log_entry.rs`) now uses `usize`-based `Information` and returns units directly from `parse_size_to_bytes`. This harmonizes with the rest of the codebase’s sizing.
- Top-level integration tests tied to the old API were removed; tests have been rewritten inline with modules, including new alignment and ordering tests.

## Completion summary

- The refactor consolidates a clean three-phase pipeline: describe (logical), plan (physical), and execute/assemble (reader + writer).
- It fixes correctness issues around alignment and set ordering, removes mixed concerns from core types, and hardens Windows IOCP usage with documented invariants.
- It establishes patterns (policy toggles, predecessor-aware lookups, repr(C) carrier types) that are broadly reusable in similar I/O-heavy code.



===

from gemini

This is a significant and well-executed refactoring that accomplishes two primary goals:
1.  It fully adopts the Bevy game engine as an application framework, moving beyond just using its Entity-Component-System (ECS) library.
2.  It dramatically improves the design of the low-level disk reading logic by separating concerns, increasing robustness, and improving clarity.

### 1. Summary of the Refactor

The entire refactor transitions the project from a simple command-line tool into the foundation of a Bevy-based application. The core logic for reading the Master File Table (MFT) from a physical drive has been overhauled. Previously, a single module was responsible for defining a plan of disk reads *and* executing them using complex Windows-native I/O Completion Ports (IOCP). This monolithic logic has been broken down into a clean pipeline: a logical plan describing the file structure, a physical plan for disk reads, and a dedicated `PhysicalReader` to execute the reads. The process of writing the final MFT file is now guided by the logical plan, making it far more robust and correct.

### 2. Analysis of Changes

*   **Framework Adoption (`Cargo.toml`, `src/bevy/main.rs`):**
    *   The dependencies in `Cargo.toml` are changed from `bevy_ecs` and `bevy_reflect` to the full `bevy = "0.17.0-rc.1"` crate.
    *   The `Cargo.lock` file shows a massive increase in dependencies, pulling in the entire Bevy engine stack (windowing, tasks, assets, etc.).
    *   The entry point in `src/bevy/main.rs` is transformed from a simple `World::default()` into a proper Bevy application (`App::new()`, `app.add_plugins(...)`, `app.run()`).

*   **Modular Application Logic (`src/bevy/sync_dir.rs`):**
    *   A new file introduces `SyncDirectoryPlugin`, a standard Bevy pattern for modularizing application logic.
    *   It defines a `SyncDirectory` `Resource` to hold application state and uses Bevy's `IoTaskPool` to asynchronously load this state, demonstrating an idiomatic use of the new framework.

*   **I/O Logic Abstraction (`src/read/`):**
    *   The complex Windows IOCP logic, previously located inside `physical_read_plan.rs`, has been extracted into two new files: `physical_reader.rs` and `active_physical_read_request.rs`. This isolates the platform-specific, `unsafe` implementation details from the rest of the code.
    *   `tests/physical_rapid_reader_tests.rs` was deleted because the logic it tested in `PhysicalReadPlan` was moved and redesigned.

*   **Improved Data Integrity and Correctness:**
    *   The function `read_physical_mft` in `src/mft/mft_physical_read.rs` now returns a tuple of `(LogicalReadPlan, PhysicalReadResults)`.
    *   The `write_to_file` method in `src/read/physical_read_results.rs` now requires this `LogicalReadPlan` to accurately reconstruct the output file. This is a critical change that eliminates fragile assumptions about data layout.

### 3. Changed Data Structures and Mental Models

The refactoring introduces a profound shift in the mental model for how file reading is planned, executed, and reassembled.

#### The "Before" Model: A Monolithic Plan

Previously, the mental model was centered around a single, multi-purpose `PhysicalReadPlan` structure.

*   **Data Structure:** `requests: Vec<PhysicalReadRequest>`. A simple, unsorted vector of read operations.
*   **Mental Model:** This "plan" was a combined blueprint and factory. It described *what* to read and also contained the complex, monolithic logic to execute the reads using IOCP. The final step, `write_to_file`, received a flat list of data chunks and had to infer where to write them based on offsets, a process that was fragile, especially with disk read alignments.

#### The "After" Model: A Clean Data Pipeline

The new model is a clear, multi-stage pipeline where each component has a single, well-defined responsibility.

*   **Data Structures:**
    *   `LogicalReadPlan`: Uses a `BTreeSet<LogicalFileSegment>` to store a unique, sorted list of a file's segments, including sparse (zero-filled) areas.
    *   `PhysicalReadPlan`: Also uses a `BTreeSet<PhysicalReadRequest>` to maintain a unique, sorted list of raw reads to perform on the disk.
    *   `PhysicalReader`: A new struct that encapsulates the state of the I/O operations (file handles, completion ports, etc.).
    *   `ActivePhysicalReadRequest`: A new struct that wraps the Windows `OVERLAPPED` structure, managing the lifecycle of a single in-flight I/O request.

*   **Mental Model:** The process is now a logical flow:
    1.  A `LogicalReadPlan` is created, serving as the "source of truth" for the file's layout.
    2.  This is converted to a `PhysicalReadPlan`, which is a pure data object describing *what* to read from disk.
    3.  The `PhysicalReader` consumes the `PhysicalReadPlan` and executes it, hiding all the complexity of asynchronous I/O.
    4.  The final `write_to_file` function uses the original `LogicalReadPlan` as an explicit map to correctly assemble the physically read chunks into the final, logical file output. This is a shift from *inferring* layout to *following* an authoritative map.

### 4. Why is the New Approach Better?

The new approach is a significant improvement for several reasons:

*   **Separation of Concerns:** The most critical improvement. The `PhysicalReadPlan` is now a simple data container. The complex, platform-specific IOCP execution logic is isolated in `PhysicalReader`. This makes the code vastly easier to understand, test, and maintain.
*   **Correctness and Robustness:** The old `write_to_file` was fragile. It had to manually calculate offsets to account for reads that were over-aligned for performance. The new implementation is far more robust:
    ```rust
    // src/read/physical_read_results.rs
    pub fn write_to_file(
        &self,
        logical_plan: &LogicalReadPlan, // <-- Explicit "map"
        output_path: impl AsRef<std::path::Path>,
    ) -> eyre::Result<()> {
        // ...
        for logical_segment in logical_plan.segments.iter() {
            // ... uses the logical_segment to find and place the correct physical data
        }
        // ...
    }
    ```
    By using the `LogicalReadPlan` as an authoritative guide, it correctly assembles the output file without ambiguity, flawlessly handling sparse sections and fragmented data.

*   **Clarity and Intent:** The choice of data structures makes the intent clearer. Using `BTreeSet` instead of `Vec` for the plans guarantees that read segments are always sorted and unique, eliminating the need for manual sorting and preventing duplicate work.

*   **Scalability:** By fully adopting Bevy, the project now has a powerful, data-oriented framework for future development. Adding UI, managing complex state, and handling user input is now much more straightforward than it would have been with a custom application loop.

### 5. Lessons Learned & Re-application

This refactor provides several valuable lessons that can be applied to other projects:

1.  **Separate the "What" from the "How":** Look for classes or modules that both define a set of tasks and contain the complex machinery to execute them. This is a "Monolithic Executor" smell. The solution is to separate these into a pure data "Plan" and an "Executor/Reader/Processor" class, as was done with `PhysicalReadPlan` and `PhysicalReader`.

2.  **Make Implicit Contracts Explicit:** The old `write_to_file` implicitly relied on the caller to handle the relationship between logical and physical reads. The new version makes this relationship an explicit parameter (`logical_plan: &LogicalReadPlan`). If a function needs a "map" or "guide" to interpret its other inputs correctly, pass that guide as an explicit argument.

3.  **Encapsulate `unsafe` Complexity:** The low-level IOCP logic is inherently complex and involves `unsafe` code. By isolating it within `PhysicalReader` and `ActivePhysicalReadRequest`, the rest of the application can interact with a safe, high-level API, minimizing the surface area for bugs.

4.  **Choose Data Structures that Enforce Invariants:** The switch from `Vec` to `BTreeSet` is a prime example. The `Vec` required manual sorting (`self.requests.sort_by_key(...)`). The `BTreeSet` guarantees sorted order and uniqueness automatically, making the code simpler and its invariants clearer. When you find yourself repeatedly sorting a collection, consider if a sorted data structure is a better fit.