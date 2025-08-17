An excerpt from a conversation I found helpful
---
Below is a concise, structured deep‑dive you can keep handy while rewriting around a raw &[u8] representation.

MFT record high‑level
- Fixed-size record (commonly 1024 bytes; can be 4096 on some volumes).  
- Starts with a Multi‑Sector Header (the “FILE” / “BAAD” / zeros part).  
- Followed by Update Sequence Array (USA) region.  
- Then attribute records, each with its own header, ending with type 0xFFFFFFFF.

Multi‑Sector Header (offsets within the record)
0x00 (4)  Signature: b\"FILE\" | b\"BAAD\" | zeros (unused slot)  
0x04 (2)  usa_offset (byte offset to start of USA)  
0x06 (2)  usa_size (count of 2‑byte elements; first is the Update Sequence Number, rest are replacement words)  
0x08 (8)  $LogFile Sequence Number (LSN)  
0x10 (2)  Sequence number (for stale reference detection)  
0x12 (2)  Hard link count  
0x14 (2)  First attribute offset (start of first attribute header)  
0x16 (2)  Flags (bit 0 allocated, bit 1 directory)  
0x18 (4)  Used size of this record (bytes actually used)  
0x1C (4)  Allocated size of this record (== record size)  
0x20 (8)  Base record reference (if this is an extension)  
0x28 (2)  Next attribute ID  
0x2A (2)  Padding / alignment  
0x2C (4)  MFT record number (stored as 4 bytes on disk; many parsers widen to u64)  
(After that, typically padding up to first_attribute_offset.)

Update Sequence Array (Fixups)
Purpose: Detect torn multi‑sector writes (power loss mid‑update). The record is conceptually divided into 512‑byte strides.  
Layout at usa_offset:
- Element 0: USN (2 bytes) – the sentinel value written into the last 2 bytes of every 512‑byte stride before writing to disk.
- Elements 1..N: Original last 2 bytes of each stride (N = usa_size - 1).  
Apply algorithm (must be done before trusting trailing bytes of each stride):
1. Read USN (sentinel).  
2. For each stride i (0-based), compute stride_end = (i+1)*512, look at bytes [stride_end-2 .. stride_end).  
3. If those 2 bytes != USN → corruption → mark invalid (signature may be switched to BAAD by NTFS on disk after detection; you just report failure).  
4. If they match, replace them with the i-th replacement pair from the array (elements 1..).  
Result: You reconstruct the original end-of-sector bytes, and know whether the record passed integrity (your valid_fixup flag).

Signatures
- FILE: Normal, attempt fixup; if fixup fails you can still keep bytes but mark invalid.  
- BAAD: NTFS previously detected a failed multi-sector write. Treat contents as unreliable; usually skip.  
- 0x00000000: Unused / free slot.

Attribute record header (common 16 bytes)
Offset (relative to attribute start):
0x00 (4)  Type (e.g. 0x10 STANDARD_INFORMATION, 0x30 FILE_NAME, 0x80 DATA, 0xFFFFFFFF end marker)  
0x04 (4)  Total length of this attribute (header + content)  
0x08 (1)  Non‑resident flag (0 = resident, 1 = non‑resident)  
0x09 (1)  Name length (in UTF‑16 code units)  
0x0A (2)  Name offset (from attribute start)  
0x0C (2)  Flags (e.g. compressed, encrypted, sparse)  
0x0E (2)  Attribute ID  

Resident extension (when non_resident == 0):
0x10 (4)  Content size  
0x14 (2)  Content offset (from attribute start)  
0x16 (1)  Indexed flag (for FILE_NAME in directories)  
0x17 (1)  Padding  
0x18 ...  Content bytes inline (this is where a small DATA attribute’s payload lives)

Non‑resident extension (when non_resident == 1):
0x10 (8)  Starting VCN  (virtual cluster number)
0x18 (8)  Last VCN  
0x20 (2)  Data runs offset (from attribute start)  
0x22 (2)  Compression unit size (power-of-two clusters; 0 = uncompressed)  
0x24 (4)  Padding / reserved  
0x28 (8)  Allocated size (bytes on disk)  
0x30 (8)  Real size (logical size)  
0x38 (8)  Initialized size  
0x40 ...  (optional: compressed size)  
(runlist begins at data_runs_offset)

Runlist (data runs)
Sequence of variable-length tuples; each starts with a header byte: high nibble = number of bytes for signed cluster offset delta, low nibble = number of bytes for unsigned length (clusters). First run’s delta is relative to 0; accumulate to get absolute LCN each time. 0x00 header terminates.

Resident vs non-resident quick rule
- If the file’s DATA attribute fits in record (<= ~700–900 bytes depending on overhead), stored resident: you find content by content_offset; no runlist.
- Larger files: DATA attribute is non‑resident: use runlist extents to map logical file contents to disk clusters.

Minimal lazy-access helpers over &[u8]
(Positions assume you have validated length)
fn sig(rec:&[u8])->&[u8;4]
fn usa_offset(rec:&[u8])->u16
fn usa_size(rec:&[u8])->u16
fn first_attr_offset(rec:&[u8])->u16
fn record_flags(rec:&[u8])->u16
fn used_size(rec:&[u8])->u32
fn allocated_size(rec:&[u8])->u32
fn record_number(rec:&[u8])->u32
fn iterate_attributes(rec:&[u8]) -> iterator over slices
fn attribute_type(attr:&[u8])->u32
fn attribute_total_len(attr:&[u8])->u32
fn is_non_resident(attr:&[u8])->bool
fn resident_content(attr:&[u8])->Option<&[u8]>
fn nonresident_runlist(attr:&[u8])->Option<&[u8]>
(teamy note: these fns are now the get_ ones)

Fixup application in your raw model
- Must be done once per record (if signature == FILE) before any attribute iteration, because runlist or resident content might span a stride boundary whose last 2 bytes are still replaced by USN.
- You can keep a small function apply_fixups(rec: &mut [u8]) -> bool that returns integrity status; skip if sig != FILE.

Edge cases / pitfalls
- Check that usa_offset + usa_size*2 <= record_len before applying.  
- (usa_size - 1) * 512 must be <= record_len (or allow last stride partially used; commonly record_len is multiple of 512).  
- Attributes may extend only up to used_entry_size, not necessarily full allocated size.  
- End marker attribute type 0xFFFFFFFF may appear before physical end.  
- Sparse/compressed DATA attributes: runlist includes “holes” (length with zero cluster delta meaning sparse) – you’d synthesize zeroes logically (rare for $MFT itself).  
- $MFTMirr duplicates first 4 records for recovery – useful if fixup fails for early records.

Putting it together (fast parse loop sketch)
1. Ensure length >= 48 (minimum header).  
2. If sig == zeros → free/unallocated: optionally skip.  
3. If sig == FILE → apply fixups; keep valid flag. If false, you may still attempt parse but mark suspect.  
4. attr_ptr = first_attr_offset; while attr_ptr + 4 <= used_size:  
   - ty = read_u32; if ty == 0xFFFFFFFF break;  
   - total_len = read_u32; bounds check;  
   - process or skip (lazy).  
   - attr_ptr += total_len.  

Why BAAD appears
- When a torn write was detected previously, NTFS may set signature to BAAD so future readers know it’s invalid without redoing fixups; you treat it as unusable (don’t apply fixups).

If you want next steps I can draft the helper functions or a test harness that validates a raw record against the current higher-level implementation—just say.