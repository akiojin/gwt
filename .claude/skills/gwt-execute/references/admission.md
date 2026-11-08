## Canonical verification admission

Only canonical `verify.run` acquires the host-wide lease, in-process for
each Heavy command. Initial `cargo build -p gwt --bin gwtd`, ordinary `cargo test`,
`cargo clippy`, `cargo build`, coverage, direct headed browser checks, and
pre-push checks do not require a verification lease. Run the RED / GREEN /
refactor commands directly.

For canonical evidence use `verify.plan` then `verify.run`. Manual
`verify.lease.acquire`, `verify.lease.hold`, and `verify.lease.extend` are
retired and return an error without acquiring or reserving a lease.
`verify.run` manages admission and waits up to `params.max_wait_secs`
(default 300, hard cap 1500) for each Heavy command. A timeout in the
first remaining command's admission writes no replacement record and preserves
any predecessor; a later timeout retains incomplete, non-PASS results.
Inspect the holder with `verify.lease.status` and retry when
the contention is resolved. There is no manual acquire loop or
fixed retry schedule. `verify.lease.release` remains available to drain a
legacy holder.

`deferred` is a temporary wait, not a verification failure or a terminal outcome.
While the verification tool process is alive, keep polling that same invocation.
After `deferred`, retry `verify.run` with the identical full requested matrix,
or declare an active wait through `issue.monitor.wait` with explicit `params.number`
for the owner Issue, `params.reason`, and `params.resume_condition`. Keep ownership
until that condition is met. On a resume notification, retry the same matrix;
do not leave the owner idle. Never use `execution.blocked` or `build.abort` for ordinary admission contention.

Retry with the identical full requested matrix, never a caller-built subset.
Automatic resume requires valid admission-deferred evidence and exact matches
for owner, session, execution authority digest, verification plan content hash,
source fingerprint, requested commands (including order and duplicates), and
headed_e2e_commands. Every preceding result must have raw PASS and no termination signal.
Failed, killed, crashed, unreadable or mismatched records start a fresh run;
registered plan/context mismatches still require `verify.plan`.
Resume references the immutable predecessor by record id/content hash, keeps
the original start timestamp (`started_at`), and copies measured per-command
headed/nextest/admission evidence. Run outstanding Light commands first,
preserve Heavy commands' original relative order, and keep artifact restoration last.
The full registered matrix and required headed Chromium evidence in dark/light
must pass before `Overall: PASS` or Ready; retained results alone are incomplete.

Heavy Cargo admission uses a bounded host slot pool (#5082), with CPU/memory
defaults and a global `[verification] slots` override. Distinct worktrees and
effective Cargo target directories may run concurrently. The same worktree or
target remains serialized, and unknown wrappers and older exclusive holders
serialize against the entire pool. Disk reservations and isolated child temp
directories protect concurrent runs. Read `verify.lease.status` for capacity,
running/available slots, every holder's ETA, and the shared FIFO queue; do not
infer the whole host state from one representative holder.

## Legacy aliases

During the transition, `$gwt-build-spec SPEC-N` and `$gwt-fix-issue #N` are
accepted only as aliases. Continue as `$gwt-execute #N` and use the mode rules
above. Do not split behavior by SPEC versus Issue once the owner number is
known.
