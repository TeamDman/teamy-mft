Teamy: so our query cli enters through QueryRuntime visit_rows_dyn which calls LocalQueryVisitor which creates a QuerySession upon which visit_rows is called or DaemonQueryVisitor where visit_rows receives results from the earlier prepare call which launched the query_stream which calls run_query_stream_in_span which uses DaemonWorker .query calling query_drive_workers which calls DriveWorker.query which sends DriveWorkerCommand which run_drive_worker receives and calls DriveWorkerState query_with_cancel which calls query_published_drive which calls visit_drive_search_index_rows which calls visit_matching_search_index_rows which calls visit_parsed_search_index_rows which calls visit_matching_row_indices.

we want something like

QueryArgs (from cli) contains QueryPlan -> QueryPlan + QueryExecutionContext -> QueryRowVisitor applied to every result

the query plan is the summary of the entire intent, where the plan is given to an executor (local or daemon) and the executor does the filtering work and in the end applies the visitor to the rows

---

## Codex summary

The current query path has two different kinds of complexity tangled together:

1. Backend orchestration complexity: CLI-local, daemon-streamed, daemon-worker, live-observed, and published-cache queries all have their own wrappers for cancellation, limits, logging, filtering, and row delivery.
2. Index execution complexity: the actual search index logic already has useful primitives, but they are exposed as low-level visitors and then reassembled in several places.

The good news is that the index layer is further along than the old search-index planning note suggests. The current `.mft_search_index` is still row-output oriented, but `ParsedSearchIndex` already provides segment views, segment postings, extension postings, trigram postings, row materialization, and scope checks. So the next refactor probably does not need to start by inventing a new query algorithm. It should start by making one query executor responsible for applying a visitor to every result selected by a `QueryPlan`, then making every caller adapt into that visitor-shaped contract.

The uncomfortable part is that there are multiple overlapping execution paths today:

- `QueryRuntime::Local` builds a `QuerySession`, which has a cached published-index path.
- `QueryRuntime::DaemonRpc` starts a streaming RPC, but the daemon currently computes a full `Vec<QueryResultRow>` before streaming it back.
- `DriveWorkerState::query_with_cancel` chooses between live-observed state and published-index fallback.
- Published fallback in `daemon.rs` uses `query_published_drive`, which calls `visit_drive_search_index_rows`.
- Local `QuerySession` has its own cached base/overlay merge and calls `visit_parsed_search_index_rows` / `visit_matching_parsed_row_indices` directly.
- `search_index_query.rs` also has base/overlay merge logic, row index collection, materialization, deleted filtering, scope filtering, and visitor plumbing.
- `LiveDriveState` rebuilds an in-memory index from projected live rows, parses it, and calls the same low-level parsed-index visitor.

That means the same policy decisions are spread across layers: deleted rows, scope, `.teamy_mft_rules`, limits, overlay precedence, cancellation, materialization, and whether rows are pushed or collected. The call graph feels wild because there is no single boundary where a `QueryPlan` becomes visits over `QueryResultRow`s.

## Cleanup goal

Create one conceptual boundary:

`QueryPlan -> QueryPlan + QueryExecutionContext -> QueryRowVisitor applied to every result`

`QueryPlan` remains the whole request: query rules, drive pattern, scope input, profile, limit, deleted visibility, and filter visibility. The execution step keeps that same plan and prepares runtime-only context around it: resolved/canonicalized scope, discovered filter rules, cancellation, cached source handles, and any future data-source helper object.

The important part is that execution stays visitor-focused. Local and daemon paths should both present the same consumer edge: apply a `QueryRowVisitor` to each matching `QueryResultRow`. A daemon implementation can use `Rx`/`Tx` internally, but that channel bridge should be transparent to callers rather than creating a separate stream-shaped query abstraction.

The executor should own matching, overlay merge, deleted/scope handling, filter-rule classification, limits, cancellation checks, and visitor application. Upper layers should choose the executor, prepare the context, and handle transport/lifecycle details.

In practice, this probably means introducing a small query executor module that can visit rows from:

- one parsed index
- a base index plus optional overlay index
- an in-memory live index
- eventually a live graph or node-oriented index, if we choose to move past path-row output

## Proposed shape

### 1. Introduce a query executor facade

Add a module along the lines of `src/query/engine.rs` with a public internal API like:

```rust
pub(crate) struct QueryExecutionContext<'a> {
    pub filter: &'a QueryRowFilter,
    pub cancel: Option<&'a AtomicBool>,
    // Optional later: source/cache helper state prepared for this execution.
}

pub(crate) type QueryRowVisitor<'a> =
    dyn FnMut(QueryResultRow) -> eyre::Result<ControlFlow<(), ()>> + 'a;

pub(crate) trait QueryExecutor {
    fn visit_rows(
        &mut self,
        plan: &QueryPlan,
        context: &QueryExecutionContext<'_>,
        visit: &mut QueryRowVisitor<'_>,
    ) -> eyre::Result<()>;
}
```

Then give the executor/source helpers methods such as:

```rust
visit_parsed_index(plan, context, parsed_index, visit)
visit_base_overlay(plan, context, base_parsed, overlay_parsed, visit)
visit_published_drive(plan, context, drive_source, visit)
```

Names can change, but the ownership should be clear: `QueryPlan` is the request, `QueryExecutionContext` is prepared runtime state around that request, and `QueryRowVisitor` is the consistent output edge. Callers own lifecycle and transport; the executor owns matching and visitor application.

### 2. Move base/overlay merge into one place

Today base/overlay precedence exists in both `search_index_query.rs` and `query_session.rs`. The refactor should extract this into the executor and make both local sessions and daemon published fallback use the same code.

The behavior to preserve:

- overlay rows sort/merge by path
- overlay rows win when the same path exists in base and overlay
- deleted-state filtering applies before final emission
- `.teamy_mft_rules` classification applies before final emission
- limits count visited rows, not raw matched rows
- cancellation can stop candidate collection and visitor application

### 3. Put source helpers inside execution context

Keep loading details out of the matching code, but do not promote a data source into the top-level conceptual formula. If we need a helper object for mapped indexes, parsed live bytes, base/overlay pairs, or cached drive state, it should be prepared as part of `QueryExecutionContext` or hidden behind a concrete `QueryExecutor` implementation.

- `QuerySession` should cache mapped base/overlay indexes and hand parsed views or source helpers to the executor.
- `query_published_drive` should become a small adapter or disappear in favor of a shared published-drive executor/source helper.
- `LiveDriveState` should keep owning live graph refresh and in-memory index rebuilding, then hand the parsed current index to the executor.
- `DriveWorkerState` should choose live versus published fallback, but not know the mechanics of index matching.

This turns `DriveWorkerState::query_with_cancel` into orchestration instead of another query implementation.

### 4. Collapse `QueryRuntime` and `QuerySession` responsibilities

`QueryRuntime` is currently a backend selector, cancellation wrapper, session factory, daemon stream client, and visitor adapter. `QuerySession` is both a persistent local cache and a backend selector that can bounce back into `QueryRuntime::daemon_rpc()`.

A cleaner split would be:

- `QueryRuntime`: one-shot entry point used by CLI, responsible for Ctrl-C handling and selecting local versus daemon transport.
- `QuerySession`: local published-index cache only. No daemon backend variant.
- daemon client path: a separate adapter that applies the caller's visitor to rows arriving from RPC.

That removes the recursive-feeling relationship where a daemon `QuerySession` delegates back to `QueryRuntime`.

### 5. Keep daemon channels as a transport detail

The daemon `query_stream` API streams over RPC, but inside the daemon it still waits for `worker.query(...) -> Vec<QueryResultRow>` before sending rows. That means cancellation can stop before or after the worker query, but not during most row discovery except where the worker happens to poll the flag.

The abstraction does not need to split into visitor mode locally and stream mode in the daemon. The daemon path can implement the same visitor-shaped edge with `Rx`/`Tx` in the middle:

```text
client-side visitor <- rows_rx <- RPC <- rows_tx <- daemon-side visitor <- executor
```

There are two reasonable choices:

- Short term: keep collecting in the worker, but make the RPC adapter the only place that speaks in channel terms.
- Better: make drive workers apply a daemon-side visitor as they query, with that visitor forwarding rows over `Tx`.

I would not do true end-to-end incremental emission first unless query latency or memory makes it urgent. The duplication cleanup is the higher-leverage move.

### 6. Keep `search_index_query.rs` focused or retire it

Right now `search_index_query.rs` contains a mix of:

- candidate matching
- scope matching
- parsed row visiting
- index file loading
- base/overlay merge
- deleted filtering
- materialization helpers

After the executor extraction, it should either become the parsed-index execution module or be split into:

- `query/index_match.rs`: candidate row index calculation from `ParsedSearchIndex`
- `query/engine.rs`: query execution, filtering, limits, cancellation, visitor application
- `query/source.rs`: published-drive loading and cache adapters

The exact filenames matter less than making each layer answer one question.

## Suggested implementation order

1. Add executor tests around the behavior we cannot regress: deleted filters, scope filters, profile/rules filtering, limits after filtering, cancellation, and overlay precedence.
2. Extract the base/overlay merge from `QuerySession` and `search_index_query.rs` into one helper without changing public behavior.
3. Change `query_published_drive` to use the same helper as `QuerySession`.
4. Change `LiveDriveState::query_indexed_with_cancel` to call the same parsed-index executor function instead of directly calling `visit_parsed_search_index_rows` with custom limit/filter logic.
5. Remove `QuerySessionBackend::DaemonRpc`; keep `QuerySession` as the reusable local published-index cache.
6. Simplify `QueryRuntime` so it only handles one-shot local execution, daemon transport, Ctrl-C forwarding, and cleanup.
7. Once the ownership boundaries are clean, decide whether daemon workers should apply visitors incrementally behind the same RPC adapter.

## North-star call graph

Local CLI:

```text
query_cli -> QueryRuntime::local -> QuerySession -> QueryExecutionContext -> QueryExecutor::visit_rows -> QueryRowVisitor
```

Daemon CLI:

```text
query_cli -> QueryRuntime::daemon -> client QueryRowVisitor <- Rx/Tx RPC bridge <- daemon QueryRowVisitor <- DriveWorker -> QueryExecutor::visit_rows
```

Live drive worker:

```text
DriveWorkerState -> LiveDriveState refresh/cache -> QueryExecutionContext -> QueryExecutor::visit_rows -> QueryRowVisitor
```

Published fallback:

```text
DriveWorkerState -> published source helper in QueryExecutionContext -> QueryExecutor::visit_rows -> QueryRowVisitor
```

That gives us one matching/filtering implementation and several thin adapters. The query mechanism stays flexible and visitor-focused, while daemon transport remains an implementation detail instead of a separate conceptual model.