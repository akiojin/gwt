# Private command runtime lifetime (2026-11-02)

This slice fixes one reproduced source of private HTTP send failures. It does
not attribute every historical notification to that source.

GUI callbacks retain a shared `ProjectRuntimeClient`, but execute commands in
short-lived Tokio current-thread runtimes. Reqwest's idle pool can hand a
connection created on runtime A to a command running on runtime B. The
connection driver still belongs to A. Ending A while B awaits its response
aborts B's request even when the HTTP server received it and remains healthy.

The baseline client and release v9.147.0 (`f85ec13d`) share Git blob
`6e78c70c6f9b4fe2b100e3109058635744dea59e` and locked reqwest 0.12.28,
hyper-util 0.1.20 and Tokio 1.54.1. This source comparison does not identify
the running installed binary or establish the cause of past notifications.

The native probe uses the actual client, an authenticated inert loopback
server, and channels/barriers: complete A's Health request, receive B's Resize
request, fully drop A, then release the server response. The same probe source
SHA256 is `01a36c750c7efd5762cea3657d3bde52db64d75969378905b3c5e632129f4f04`
before and after. Each case also completes its initial cold Health request.
These are HTTP command callers, with zero Agent launches.

| Concurrent callers | Before: failed / passed | After: failed / passed | Requests received, both runs |
| --- | --- | --- | --- |
| 1 | 1 / 0 | 0 / 1 | 2 |
| 2 | 1 / 1 | 0 / 2 | 3 |
| 12 | 1 / 11 | 0 / 12 | 13 |

Before the change, exactly one caller reused A's connection in each case and
that request failed with the generic send error. Afterwards none reused it.
Request counts remain exactly one initial Health plus the requested callers;
no replay was introduced. Single elapsed samples are retained as observations,
not a throughput or GUI performance claim.

The fix configures this private client to retain zero idle connections.
Its shared configuration remains reusable, while each active connection has
the same lifetime as its caller. This avoids adding an executor or changing
six GUI call sites. It preserves the existing 120-second command deadline,
private protocol, authentication, owner effects and error propagation.
The WebSocket presence connection uses a separate implementation.

The single regression waits for the second authenticated request before ending
the first caller's runtime. It then releases the response and requires success
without replay. The old client failed that assertion; the fixed client passed.
It uses channel receipts and the common hang guard, with no short elapsed-time
ordering assertion. The selected existing `project_runtime::` suite passed
56 tests. Intended RED/GREEN paths join their callers and server before the
result assertion; this does not claim explicit joins on every fixture panic.

A separate raw reqwest control records only phase and boolean error categories.
The induced failures are in `send`, with `is_request=true` and
`is_timeout/is_connect/is_body/is_decode=false`. The no-idle control has no
failures. These flags describe the controlled fixture, not past log strings.
The sources, binaries, raw test outputs, library-control outputs and hashes
are preserved in [the evidence artifact](rpc-runtime-lifetime.json).

Backend-first canonical record `vrr-d833d31fddff43a7bb2e36aad4cf2f13`
passed 13/13 commands, with `all_passed` and `plan_covered` true. It includes
strict workspace Clippy, formatting, both binaries, 101 Rust tests (one
existing helper ignored), and the managed-browser contract's seven rejected
invalid observations. The record file SHA256 is
`5ef73c5c13c68bf228136e22a1894d9f6d25ca274e632b0311990b70e865d6a1`.
Execution status immediately afterwards was active/bound/fresh/autonomous.
This checks the dirty fix on `f2b3a1d1bc574695eaf5c33fc92da978fccf3572`,
before this checkpoint's documentation and commit. It is an intermediate
backend result, with zero actual headed passes, rather than final-head Ready
evidence. Required PR coverage is unmeasured; no threshold failure is claimed.

The existing global log segment from 05:10 to 05:26 UTC contains 71 private
send warnings from the static PM wake prompt-injection failure site. Its
segment offsets/hash and per-row hashes are retained without exporting input
payloads or credentials. Those rows have no request/phase correlation to the
three pictured Agent Error notifications. Their absence of a timeout literal
does not classify the original transport error.

The inert server does not exercise the production owner mailbox, actual
2/12 Agent operation, GUI delivery, installed rollout or CPU/RSS/fleet budgets.
EO3/EO4, the original notifications, actual headed dark/light verification,
final-head canonical verification, required CI/coverage and fetched base
delivery remain open. The existing runtime/ledger gates are preserved.
The post-backend doctor remains current/converged/normal, but returns
`unknown` with `reasons=[ledger_unavailable]`. Shared installed GUI PID 22480
is still the same launch. No registration, ledger edits or shared GUI stops
were used to turn those prerequisites into a pass.

User Verification Result: n/a (autonomous).
Agent Visual Check: not run (existing runtime/ledger gates remain in force).
