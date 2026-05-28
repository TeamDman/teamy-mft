# Daemon Service Console Attempt

## Context

We wanted a way to see daemon stderr/stdout directly while debugging the
Windows service. The motivation was to make daemon startup stalls and early
RPC delays visible without waiting for a client-side log pump to connect.

The attempted design added a `--daemon-console` install option. Installing with
that flag wrote a service environment value:

```text
TEAMY_MFT_DAEMON_ALLOC_CONSOLE=1
```

On service startup, the daemon checked that environment variable, called
`AllocConsole`, replayed the in-process log buffer to stdout, and continued
normal daemon startup.

## What Happened

The daemon logged that console allocation succeeded:

```text
Console allocated, new logs will be visible here.
Daemon service debug console allocated
```

But no usable console appeared on the interactive desktop when the daemon was
started as a Windows service.

This is expected enough to make the approach unreliable. The daemon service runs
under the Service Control Manager, typically as `LocalSystem` in session 0.
Even if `AllocConsole` succeeds for that process, the console is allocated in
the service's non-interactive session, not as a visible terminal attached to the
developer's desktop session.

## Why We Removed It

The feature made the install path and service configuration more complex while
not solving the debugging problem. It also created an install-time preference
that was hard to reason about because visibility depended on Windows service
session behavior rather than teamy-mft itself.

The removed pieces were:

- `--daemon-console` on `install` and `service install`
- service `Environment` registry writing for
  `TEAMY_MFT_DAEMON_ALLOC_CONSOLE`
- daemon startup `AllocConsole` hook
- helper/test code for service `REG_MULTI_SZ` environment encoding

## Preferred Direction

Use daemon log forwarding and explicit observability surfaces instead:

- forward daemon startup logs through the first client ping so startup progress
  is visible in the caller's terminal
- keep daemon logs correlated with RPC correlation IDs
- filter noisy dependency logs before forwarding to the client
- keep span transition events available to profiling/tracing layers while
  hiding them from stderr
- use Tracy or another profiler for timeline visibility instead of relying on a
  service-owned console

If a live interactive daemon log view is still needed, build it as an explicit
client command that attaches to the daemon log stream over IPC. That works
within the user's existing terminal session and avoids Windows service desktop
isolation.
