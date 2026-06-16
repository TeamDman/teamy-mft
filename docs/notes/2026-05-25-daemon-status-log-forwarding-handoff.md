# Daemon Status Log Forwarding Resolution

## Summary

`teamy-mft daemon status` now prints correlated `[daemon]` ping logs forwarded from the machine daemon to the client over the Vox daemon-log side channel.

The root cause was not Windows service elevation, stale daemon binaries, or channel lifetime. The root cause was that `DaemonLogEvent` contained `CorrelationId(Uuid)`, and Vox's dynamic channel payload encoder rejected `Uuid` with:

```text
[vox_jit::slow_path] to_vec_dynamic (non-opaque) failed for shape=Uuid: unsupported type: Uuid
```

That send failure closed the Vox connection before the client could receive daemon log events.

## Fix

Keep `CorrelationId` as the domain type:

```rust
pub struct CorrelationId(pub Uuid);
```

Add a wire proxy for streamed daemon log events:

```rust
pub struct DaemonLogWireEvent {
    pub correlation_id: Option<String>,
    // other fields mirror DaemonLogEvent
}
```

`DaemonLogEvent` remains the in-process log event with `Option<CorrelationId>`. The daemon log forwarder converts `DaemonLogEvent` to `DaemonLogWireEvent` before `logs_tx.send(...)`, and the client-side log drain converts `DaemonLogWireEvent` back to `DaemonLogEvent` before rendering.

Relevant file:

- `crates/teamy-mft-daemon-rpc/src/lib.rs`
- `src/machine/daemon_log.rs`

## Regression Harness

Added an in-memory Vox regression test that avoids SCM, named pipes, service install, and admin elevation:

- `tests/vox_daemon_log_channel.rs`

The test defines a tiny Vox service with the same shape as the daemon log path:

```rust
async fn ping(&self, logs: vox::Tx<DaemonLogWireEvent>) -> String;
```

It verifies both important client orderings:

- receiver drains while the RPC call is in flight
- receiver starts after the RPC call returns

Both pass after moving the Vox channel payload to the proxy shape while keeping `CorrelationId(Uuid)`.

## Service Verification

After reinstalling:

```powershell
.\install.ps1
teamy-mft install --force --sync-dir G:\Programming\Caches\MFT_FILES\
teamy-mft daemon status --debug
```

Observed output includes the desired daemon log:

```text
[daemon] info ping <correlation-id> teamy_mft::machine::daemon: Daemon pong (service_name=teamy-mft-daemon)
```

The output also currently includes duplicate synthetic `enter_span` / `exit_span` rows. That is likely because tracing span enter/exit can happen more than once as instrumented async futures are polled. The important functional issue is fixed: daemon event payloads now cross the Vox channel and render on the client.

## Other Related Changes

Build freshness is now more reliable:

- `install.ps1` sets `TEAMY_MFT_BUILD_UNIX_MS` before `cargo install`
- `build.rs` consumes that env var and declares `cargo:rerun-if-env-changed=TEAMY_MFT_BUILD_UNIX_MS`

This prevents local installs from reusing stale `BUILD_UNIX_MS` values when `build.rs` itself does not rerun.

Several other daemon status/service changes were already in the worktree before this resolution, including build metadata reporting, SCM delete wait behavior, and status output improvements. Review `git diff` carefully before committing.

## Verification

These checks passed:

- `cargo test --test vox_daemon_log_channel -- --nocapture`
- `.\check-all.ps1`
- local service reinstall and `teamy-mft daemon status --debug`

No commit was made.
