## Heavy verification serialization (SPEC #3576)

Only canonical `verify.run` acquires the host-wide lease, in-process for
each Heavy command and releases it immediately afterward. Light commands
do not hold the lease, even within a mixed matrix. Initial `cargo build -p gwt --bin gwtd`, ordinary `cargo test`,
`cargo clippy`, `cargo build`, coverage, direct headed Playwright, and
pre-push checks do not require a verification lease. Run them directly;
they do not replace the canonical evidence required for completion.

Register the matrix with `verify.plan`, then execute it through
`verify.run`. Manual `verify.lease.acquire`, `verify.lease.hold`, and
`verify.lease.extend` are retired and return an error without acquiring or
reserving a lease. Do not wrap Cargo commands or `verify.run` in a manual
lease acquisition loop.

`verify.run` owns its admission and bounded wait through
`params.max_wait_secs` (default 300, hard cap 1500). While waiting,
`verify.lease.status` lists the run under `pending`. A timeout in the
first remaining command's admission writes no replacement record and preserves
any predecessor; a later timeout retains incomplete, non-PASS results. Inspect the
reported holder and wait reason, then retry when contention is resolved;
there is no fixed retry schedule. A deferral is not a spent attempt and
there is no attempt cap: the refusal keeps your turn reserved
(`next_turn_reserved: yes`, `queue_position`), so keep rerunning
`verify.run` (a resident wait) while the holder makes progress. The lease
is released after every Heavy command and a holder's next command queues behind you. A
running `verify.run` publishes its progress, so the refusal and
`verify.lease.status` show `remaining_batches` (commands left in the
holder's run) and `estimated_remaining_ms`. If a holder persists without a live
verification workload, report its run / PID and timing evidence to the
PM. `verify.lease.release` remains available to drain a legacy holder
without killing its process.

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

### gwtd bootstrap order

In a checkout that builds gwtd from source (the gwt repository itself), the
first `cargo build -p gwt --bin gwtd` is a lease-free bootstrap step, never a
heavy verification command. The order is build → `verify.plan` → `verify.run`:
build the checkout binary without holding or waiting for any lease, and only
then run canonical verification through it.

Decide first whether the checkout binary is needed. Only operations that
execute checkout code need it: `execution.*`, `workspace.*`, `build.*`,
`verify.*`, and any operation added in the checkout. Read-only `issue.*`,
`pr.*`, `board.*`, and `search` operations run through the resolved installed
gwtd (`GWT_BIN_PATH` / PATH). Never wait for the build or a lease just to read
Issue, PR, or Board state.

`verify.run` classifies each command before admission (Issues #4196 / #4823).
`--workspace`, `--all`, `--all-targets`, `--exclude`, multiple packages or
targets, and glob selectors widen scope and remain Heavy. `--all-features`
alone does not widen scope. A whole-crate test run, including unfiltered
`-p <crate> --lib`, remains Heavy. One named target (`--test <name>`,
`--bin <name>`, `--example <name>`), or `-p <crate> --lib` with a test filter,
is Light. Bare `--lib` is not narrow in a virtual workspace.
`cargo fmt`, `cargo metadata`, markdownlint, and `cargo doc --no-deps` are
Light. Unknown commands and value-taking Cargo global options remain
conservatively Heavy. Environment assignments are unwrapped before classification.
The run reports its overall scope and the command requiring Heavy admission,
but Light commands can start while another worktree holds the lease. Several
worktrees may run Light commands concurrently at their existing normal priority.

## Stop Conditions

Stop and surface a blocker to the caller when:

- `git merge-base HEAD origin/develop` (and `origin/main`) both fail —
  cannot establish a baseline.
- Tooling bootstrap exhausts auto-install options (see
  [references/tooling-bootstrap.md](tooling-bootstrap.md)) and emits `failed: tooling-missing`.
- A required visual / UI snapshot diff is non-zero in `--mode full` /
  `--mode pre-pr` — agent must triage rather than silently regenerate
  snapshots.
- The agent attempts to invoke Playwright (or any other browser UI runner)
  for a surface not classified as a UI surface under
  [references/surface-taxonomy.md](surface-taxonomy.md) (including its acceptance-aware
  escalation). This is a contract violation, not a runtime error.
- The User Verification phase is required but no platform question tool
  and no plain-text channel are available to ask the user.

## Project-local AGENTS.md takes precedence

This skill is distributed into every gwt-managed worktree. The host project
defines its own testing reality. When the project root holds an AGENTS.md /
CLAUDE.md / README with a "ローカル検証" / "Testing" / "Verification" section,
the agent must read those instructions and prefer them over the generic
matrix in this skill. Use the generic matrix only to fill gaps.

This mirrors the broader gwt rule: gwt's own AGENTS.md is project-local to
the gwt repo, and any other project gwt opens has its own AGENTS.md as the
authority.

## gwtd resolution

Before invoking `gwtd` from this skill or its references, resolve `GWT_BIN`
first: executable `GWT_BIN_PATH`, then `command -v gwtd`, then
`$GWT_PROJECT_ROOT/target/debug/gwtd` or `./target/debug/gwtd`. If none
exists, stop with `gwtd not found`.

## References

- [references/surface-taxonomy.md](surface-taxonomy.md) — abstract surface classification and
  per-project path-pattern examples.
- [references/runner-detection.md](runner-detection.md) — manifest → test-runner detection
  table (Rust / Node / Python / Go / .NET / Unity / Ruby / Java / Make /
  generic).
- [references/user-verification-guide.md](user-verification-guide.md) — 4-step 導線 generation guide
  with project-type-specific launch patterns and Check Items rules.
- [references/test-matrix.md](test-matrix.md) — example matrix for the gwt repo itself,
  referenced as a worked example of how to specialize the generic
  contract.
- [references/playwright-runbook.md](playwright-runbook.md) — operational details for browser /
  WebView UI runners (Playwright is the canonical example; the same
  pattern applies to Cypress / Selenium / WinAppDriver / Unity Editor
  headed).
- [references/tooling-bootstrap.md](tooling-bootstrap.md) — best-effort installer contract and
  the `failed: tooling-missing` shape.

## Chain Suggestion

On `Overall: PASS`, the caller proceeds:

- `gwt-build-spec` Phase 3 → Phase 4 (PR Flow via `gwt-manage-pr`), provided
  `User Verification Result ∈ {confirmed, n/a, n/a (autonomous),
  deferred (autonomous execution), skipped(<reason>)}`. The legacy deferred
  value follows the autonomous compatibility rule above.
- `gwt-manage-pr` → PR create / update, provided the same User Verification
  Result gate is satisfied.
- Manual invocation → return the evidence bundle to the user.

On `Overall: FAIL` (including `User Verification Result: rejected(...)`),
the caller stays in their current phase and routes the failure for repair
(typically back into the TDD loop or `gwt-discussion`).
