# Sync resource use

`teamy-mft sync --parallel 1` is the default: one drive index is built at a time,
with one shared Rayon indexing worker. `--parallel 2` allows two simultaneous
index builds and two shared indexing workers. The option applies to direct and
daemon sync. `--parallel 0` lets Rayon choose its automatic worker count; the
resolved count also limits concurrent index builds. Drive acquisition remains
parallel across selected drives, independently of this indexing limit.

Acquisition streams completed 1 MiB reads into a temporary cached snapshot and
replaces the previous `.mft` only after successful reads, flush and publication.
At the default `TEAMY_MFT_MAX_IN_FLIGHT_IO=32`, acquisition retains approximately
32 MiB of payload per active drive, plus a completed chunk and plan metadata,
instead of retaining every drive's entire MFT. Reads are queued in physical
offset order; writes reconstruct logical order, including zero-filled gaps.
Lower `TEAMY_MFT_MAX_IN_FLIGHT_IO` if parallel drive I/O still affects responsiveness.
An unset value or `0` selects the automatic I/O depth of 32. Malformed, negative,
or out-of-range values return a configuration error.

Indexing reads the resulting cached snapshot into one owned buffer, applies
fixups, extracts paths, then releases that buffer before serializing the index.
This is ordinary buffered I/O, not zero-copy: it adds a cached-file read and can
use the Windows file cache. A large single index still needs memory for paths
and index tables. Snapshot and index replacement are individually atomic; they
are not one atomic generation transaction. Existing skip/overwrite and USN
checkpoint publication rules still apply.

Direct and daemon sync require compatible client and daemon builds. Rebuild
and restart both when changing the sync RPC schema; building the sources does
not automatically update a running service.


## Publication and concurrent processes

Snapshot staging is created with a protected DACL granting the process user,
SYSTEM and Administrators access; it does not inherit broad directory read
grants. It stays private through acquisition and flush. After a successful
flush, the intended destination access policy is applied, cancellation is
checked again, and the file is renamed into place. Existing-file DACLs are
preserved. For first publication, Windows derives a new non-container file DACL
from the parent and creator's effective token; directory-only ACEs are not copied as file
grants. Cancellation is cooperative:
an arrival after that last check can race the rename syscall.

Cooperating full-sync processes hold an exclusive OS handle on `.full-sync.lock`
for acquisition, indexing and sidecar publication. A second full sync for the
same cache fails rather than replacing the pathname being indexed by the first.
The small lock file remains; closing the handle (including on process exit)
releases the lock. No stale-lock-file removal is needed.

This lock is not a generation transaction. Readers do not take it, and path-sync,
live overlay writers, older binaries and external tools do not participate.
Snapshot, base index, overlay and checkpoint are still separate publications;
a crash or partial multi-drive failure can leave mixed generations. Versioned
artifact names plus an atomic generation manifest, and a policy shared by all
writers/readers, would be required to establish tuple-level crash consistency.
