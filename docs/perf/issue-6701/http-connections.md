# Private HTTP connection reuse

## Scope and plan

Issue #6701 is an implementation gap in the existing private project transport.
The starting HEAD, `5839e35f8c452d14bdb073143a09df68ae21cbed`, matched fetched
`origin/develop`; no source commits or PR from an earlier attempt remained.

Goal: reuse private HTTP connections across callers without reintroducing
Issue #5390's caller-runtime shutdown failure. Acceptance tests must cover
both connection reuse and that existing failure with the real Rust client.
Authentication, request timeouts, error propagation, WebSocket presence,
GUI call sites and polling intervals stay unchanged. This slice does not
restart the shared installed application or change OS networking settings.

Three options were considered: simply enable the pool, move every GUI caller
to one executor, or give the HTTP transport a process-lifetime executor.
The first reintroduces the proven runtime-lifetime bug; the second spreads
the change across unrelated callbacks. A shared client and one persistent
HTTP worker keep the fix inside `project_runtime/client.rs`.

## Attribution and baseline

The Windows loopback TIME_WAIT snapshot during investigation recorded:

| Remote port | TIME_WAIT | Listener PID | Endpoint |
| --- | ---: | ---: | --- |
| 52498 | 5,775 | 18324 | Private project transport |
| 52502 | 5,513 | 18284 | Private project transport |
| 61361 | 3,179 | 29440 | Private project transport |

These three destinations account for 14,467 connections. Listener PIDs were
read using `Get-NetTCPConnection -State Listen`; each corresponding persisted
daemon endpoint's `project_transport_url` matched the port. Credentials were
excluded from the evidence. All three listeners used the installed `gwtd.exe`.
This snapshot describes the installed system, not the checkout binary.

`ProjectRuntimeClient::command_with_timeout` sends all private commands to
`/private/project/command`. Its client explicitly sets
`pool_max_idle_per_host(0)`. That setting was added by commit `2f8cf7a1b` to
avoid the [documented caller-runtime shutdown failure](../issue-5390/rpc-runtime-lifetime.md).
Removing that setting alone would regress that fix.

The GUI retains/clones clients but runs input, resize, snapshot and PM wake
commands on temporary Tokio runtimes. The owner subscriber also uses this
client for its two-second Health interval. No evidence justifies changing
that health frequency. Daemon RPC uses named pipes or Unix sockets rather
than HTTP. The separate hook-live listener is not one of these three ports.

## Controlled results

The shared transport now owns one persistent Tokio worker and one reqwest
client with at most eight **idle** connections per host. Active requests
are not capped by that idle limit. Each request completes its send and body
decode on the persistent runtime. A per-call JoinSet preserves cancellation
when the caller drops the request future. No retry was added.

The existing regression was extended rather than creating another fixture.
It completes caller A's request, receives caller B's request, fully drops A's
runtime, then releases B's response. Sixteen new clients and their clones
subsequently use fresh caller runtimes: 34 authenticated requests in total.

| Same regression workload | Before | After |
| --- | ---: | ---: |
| Requests received (no replay) | 34 | 34 |
| Server-observed TCP connections | 34 | 1 |
| New fixture TIME_WAIT socket pairs | 34 | 1 |
| Connection-count assertion | FAIL (expected RED) | PASS |

The Windows TIME_WAIT samples use `netstat -an -p tcp` immediately before
and after running the compiled regression alone. Existing socket pairs are
subtracted. New traffic to the three installed project ports is retained
separately in [the measurement evidence](http-connections.json), rather
than attributed to the fixture. The before sample started at
2026-11-03T15:52:52Z and the after sample at 2026-11-03T15:55:50Z.
They measured only the fixture's short run, not a steady-state system budget.

The initial, unextended runtime-shutdown regression passed on the baseline.
The extended test failed on the baseline with 34 connections, then passed
with one connection after the fix. All 56 existing tests selected by
`cargo test -p gwt --lib project_runtime:: -- --nocapture` passed after the
fix. Independent review found no blocking issue in initialization, driver
lifetime, authentication, timeout, cancellation or error propagation.

## Measurement boundary

The regression uses an inert authenticated loopback server and the production
client. Server-observed peer addresses measure distinct TCP connections.
They are connection-reuse evidence, not an observation of three running agents.

The Issue's target of at most 500 TIME_WAIT connections during one hour with
three actual agent windows requires a run of the updated application. The
shared installed application has not been restarted by this task. A short
fixture run or an unchanged installed-system census must not mark that
operational acceptance criterion complete.
