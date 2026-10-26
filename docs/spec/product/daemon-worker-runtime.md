# Daemon Worker Runtime

This specification covers the runtime coordination behavior of the machine
daemon and its per-drive workers.

## Snapshot Sync

dwrk[sync.skip.preserves-snapshot-state]
With `--if-exists skip`, sync must retain the MFT snapshot, overlay, and checkpoint
associated with an existing base index. It may rebuild a missing base index from
a retained MFT snapshot, but must preserve that snapshot's overlay and checkpoint,
including their absence. Only a freshly read MFT with a newly built base index may
reset the overlay and establish a new journal checkpoint.

## Worker Modes

dwrk[worker.mode.live-observed]
The daemon runtime must support a live-observed per-drive worker mode that can
load live drive state, observe journal-backed refreshes, and flush published
artifacts when needed.

dwrk[worker.mode.published-index-only]
The daemon runtime must support a published-index-only per-drive worker mode
that answers queries from published search indexes without requiring live drive
state or journal observation.

## Query Serialization And Cancellation

dwrk[worker.query.serialized-per-drive]
Each drive worker must serialize query, refresh, and flush decisions through a
single per-drive worker loop.

dwrk[worker.query.cancelled-without-degrading]
If a drive query is cancelled before execution proceeds, the worker must return
cleanly without marking the drive degraded or snapshot-only.

## Fallback Behavior

dwrk[worker.live.falls-back-to-published-cache]
If live-observed query refresh fails but the published cache for the drive is
still usable, the worker must be able to answer from the published cache
instead of failing immediately.

dwrk[worker.live.queries-use-index-cache]
When a drive is loaded in live-observed mode, query execution should reuse the
current in-memory search-index cache rather than crawling projected paths one by
one as the primary matching path.

## Refresh And Warmup Policy

dwrk[worker.refresh.timeout-driven]
Live-observed workers must refresh on idle-loop timeouts so that drive state
can advance even when queries are not currently being processed.

dwrk[worker.warmup.gradual-per-drive]
The daemon runtime must warm drive workers gradually instead of eagerly loading
all warmable drives at once.
