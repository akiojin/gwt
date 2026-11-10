# Issue #5542: parent-only hook identity observations

Classification: gwt feature. Spec Status: IMPLEMENTATION-GAP under SPEC #5390.
The fetched base was `b83474be96d2d260f11a97916d7b4826f113a352`, identical to
the work branch. PR #5661 and PR #6949 were already delivered; their source
was not reimplemented. Owner AC-1 was checked and AC-2/3 were unchecked.

## Evidence and scope

The natural failure recorded in owner comment 6753044513 spent 215.194167ms
in `begin-hook-event`, before transport. Code inspection found that capture
and lease-protected revalidation each call the shared ancestry probe. Every
probe constructed a fresh sysinfo `System` and refreshed **all** process
details, although ancestry needs only parent IDs. Runtime publication and
completion repeat the fresh checks. No cached result substitutes for them.

On sysinfo 0.40.1, Windows builds process objects and queries handles/start
times for unrelated processes. macOS reads process argument/environment
blocks even with `ProcessRefreshKind::nothing()`. Linux also enumerates tasks.
These are concrete unnecessary operations in the failing aggregate stage;
the old trace does not prove how much of its particular 215ms they consumed.

Alternatives: `All` to `Some` repeats the OS process census for each ancestry
hop on Windows/macOS; cached ancestry or omitted revalidation changes the
freshness boundary. Parent-only native observations remove the unnecessary
work while keeping the existing caller contract. Windows uses one ToolHelp
parent table, macOS uses `proc_pidinfo`, and Linux reads only the parent field
of each visited `/proc/<pid>/stat`. Linux names may contain non-UTF-8 bytes.

The public result type, zero/same-PID rules, 64-hop ceiling, root termination,
fresh host/child incarnation checks and Windows recovery Job proof remain.
Missing processes do not prove ancestry. Native query failures remain errors
so ownership callers refuse and protection callers retain their guard.

## Finite paired comparison

`ancestry-probe.rs` includes the production ancestry module directly and the
previous sysinfo algorithm as its baseline. It proves the same live parent
relationship in both modes, alternates their order over 24 rounds, buffers
output until all samples finish, and uses no Session/capability or GUI.
This is an observational comparison on the shared Windows host, **not** the
owner's real 12-agent/60-minute acceptance workload.

| Mode | Samples | Median ms | p95 ms | Maximum ms | Process detail objects |
| --- | ---: | ---: | ---: | ---: | ---: |
| Baseline full sysinfo | 24 | 26.109 | 33.725 | 39.151 | 437–441 |
| Production parent-only | 24 | 13.567 | 14.034 | 16.505 | 0 |

Use the raw `ancestry-paired.jsonl` for full precision. Median elapsed time
decreased about 48%. Baseline detail counts are the observed sysinfo map
size; the parent-only zero describes its implementation (it builds only a
parent-ID map), rather than an OS-level API-call trace. Windows still takes
a process census. No wall-clock threshold or sleep-based test is introduced.

Provenance, Rust 1.99.1, x86_64-pc-windows-msvc:

- Probe image SHA256: `69e17e7f873bdc2fd93fcf1855a1bd9da454e639427aaef6be4ab45984a798e9`
- Included production module SHA256: `ddd6ae05b142c1d5e31850a5bf4363e2db62efb4894d35514b4b751c9444fa35`
- Raw JSONL SHA256 (UTF-8/LF): `3939da13af0a886e5fae679dbfff401676b6f66dcb30db7627f6e71a8b50f171`

Compile after the checkout Cargo build, selecting its matching sysinfo and
Windows 0.62.2 artifacts (ToolHelp enabled):

```powershell
$sysinfo = (Get-ChildItem target/debug/deps/libsysinfo-*.rlib | Select-Object -First 1).FullName
$windows = (Get-ChildItem target/debug/deps/libwindows-*.rlib | Sort-Object LastWriteTime -Descending | Select-Object -First 1).FullName
rustc --edition 2021 docs/perf/issue-5542/ancestry-probe.rs --extern "sysinfo=$sysinfo" --extern "windows=$windows" -L dependency=target/debug/deps -o target/hook-ancestry-production-probe.exe
target/hook-ancestry-production-probe.exe
```

## Acceptance boundary

This independently deliverable slice removes unrelated process detail work.
It does not establish the cause or repair of the separately observed inner
25ms journal/connection expiry, nor arbitrary scheduler pauses. The aggregate
200ms, live 25ms, ordinary transport 500ms, authentication, readiness, retry,
ACK-before-live and pending-journal ordering are unchanged.

The existing `issue-6842-launch-recovery-live.spec.ts` Issue #5390 case covers
real isolated checkout/provider/Monitor launch, generated SessionStart and
UserPromptSubmit hooks, exact Monitor ACK, both themes and zero browser
errors. `ancestry-browser-check.ps1` pins a copy of the checkout hook image;
the fixture uses fresh HOME, a throwaway project and owned-process cleanup.
This is regression coverage, not a deployed fleet measurement or proof of
zero dropped UserPromptSubmit notices (the original test permits idle).

Original owner AC-2/3 remain unchecked. No historical failure is rewritten,
no new finite Windows CI run is consumed on behalf of Issue #4735, and no
production fleet or installed runtime is replaced by this verification.
