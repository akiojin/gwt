# gwt-verify

Project-agnostic verification skill. `gwt-verify` is bundled into every gwt-managed
worktree, including Rust, Node, Unity, Visual Studio / .NET, Python, Go, and any other
project type gwt opens. It defines a **Generic Verification Contract** — what to verify,
how to report, and how to involve the user — without hard-coding any particular test
runner or build tool.

This skill owns "verification time" for the active project. `gwt-build-spec` Phase 3
delegates to `gwt-verify --mode full`; `gwt-manage-pr` requires `gwt-verify --mode pre-pr`
before opening or updating a PR; users may also invoke it directly through
`/gwt:gwt-verify`.

Read-only `gh` commands are allowed during verification, and allowed calls
are recorded on the shared GitHub budget ledger. Prefer gwtd JSON operations
such as `pr.list` and `issue.view` when cached data or workflow lifecycle
context is needed. GitHub mutations must use JSON-envelope operations;
direct `gh` writes do not satisfy verification, audit, or Ready PR gates.

## Contract overview

`gwt-verify` is **project-agnostic**. It does not own a fixed cargo / pnpm /
Playwright recipe. Instead it executes the following four-part contract:

1. **Change Classification** — bucket changed files into abstract surfaces
   (UI surface / interactive CLI / business logic / build-release pipeline /
   docs / config / skill-asset / external-binding). See
   [references/surface-taxonomy.md](surface-taxonomy.md).
2. **Appropriate Test Selection** — detect the project's actual test runners
   from its manifests (Cargo.toml, package.json, pyproject.toml, go.mod,
   ProjectSettings/ProjectVersion.txt, `*.sln` / `*.csproj`, Gemfile, pom.xml,
   build.gradle, Makefile) and choose the right unit / integration / E2E /
   visual commands for each changed surface. See
   [references/runner-detection.md](runner-detection.md).
3. **Test Inventory Emission** — extract the actual test names / scenarios /
   suite paths from each runner's structured output and list them in the
   evidence bundle so the user can see **what was tested**, not just a pass /
   fail count.
4. **User Verification Handoff** — after automated tests pass, present a
   surface-specific checklist plus a 4-step 導線 ("How to access": build →
   launch → navigate → observe) and ask the user to confirm or reject. See
   [references/user-verification-guide.md](user-verification-guide.md).

For every new or changed blocking gate, the verification evidence must also
cover the complete gate acceptance contract:

- an **agent-reachable recovery operation**
- **diagnostic visibility** for both the requirement and current state
- a **positive test** proving the invalid state is rejected
- a **false-positive negative test** proving legitimate work is not rejected

A gate missing any of these four elements fails verification. Warning-only
diagnostics are not classified as blocking gates.

Project-local rules always win. If the project root has an AGENTS.md / README
section describing its own testing approach (e.g., "run `make verify`",
"use Unity Editor batch mode test runner"), the agent follows that instead of
the generic matrix in this skill. The matrix here is the fallback frame.

## Modes

| Mode | When to use | Scope |
|---|---|---|
| `--mode quick` (default) | TDD loop, narrow check during implementation | Only surfaces touched by uncommitted / working-tree changes; the narrowest representative command per runner (e.g. single-package test invocation). Skip heavy integration / E2E / visual unless the diff explicitly touches them. |
| `--mode full` | gwt-build-spec Phase 3, standalone completion gate | Full matched matrix per changed surface, including integration / E2E / visual when a UI surface is in scope — by the diff, or by acceptance-aware escalation ([references/surface-taxonomy.md](surface-taxonomy.md)). User Verification Handoff is required (see below). |
| `--mode pre-pr` | gwt-manage-pr before PR create / update | Use the approved CI-backed local subset when the project supports it; otherwise use the `full` matrix. Keep acceptance tests, checks absent from required CI, and all existing visual / user-verification gates. |
| `--headed` (flag) | Manual UI / design verification | When supplied alongside any mode that runs a browser-based test runner (Playwright / Cypress / Selenium / WinAppDriver / Unity Editor headed), launch the runner in headed mode so the user can watch. Default is headless to match CI. |

Additional flag:

- `--skip-user-check` — explicitly opt out of the User Verification Handoff
  phase for non-interactive runs (CI smoke, automated regression sweeps).
  Default is **off**; `--mode full` and `--mode pre-pr` require user
  verification unless this flag is set. The reason is recorded in the
  evidence bundle as `User Verification: skipped(--skip-user-check)`.

## CI-backed pre-PR verification

A project's approved delivery policy may delegate heavyweight checks to CI.
Only checks covered by `required_status_checks.contexts` may leave the local
matrix. A job merely running in CI is insufficient: optional failures do not
prevent auto-merge. Preserve the existing Ready, review, freshness, integrity,
and Agent Visual Check gates.

For the gwt repository, register the explicit policy with:

```json
{"schema_version":1,"operation":"verify.plan","params":{"derive":true,"mode":"pre-pr","acceptance_commands":["cargo test -p gwt --test ci_contracts ci_pre_pr_contract_test::"],"commands":[]}}
```

Replace the example acceptance test with the tests that fix the current
Issue's AC. `acceptance_commands` is mandatory for a nontrivial gwt change.
`commands` adds checks that required CI does not cover, including nominated
headed E2E for UI changes. Nothing in those explicit lists is discarded.
Run the entire returned matrix through `verify.run`.

The local subset keeps formatting, changed-crate clippy, the AC tests, and
checks absent from required CI. Derivation reads live branch protection and
validates the local workflow-to-required-context correspondence. Missing
contexts, workflow jobs, or failure propagation refuse delegation with a
specific diagnostic. Repair the CI contract, or register `mode: full` and run
the complete local matrix. Other projects retain full derivation. A reduced
local PASS means the pre-PR gate passed; delivery still waits for required CI.

## Launch mode (autonomous vs interactive)

Verification behavior depends on **who launched the work**, not on how hard
the check looks. Read it from the launch record — never from the agent's
judgement, and never from ambient environment variables:

```
gwtd <<'JSON'
{"schema_version":1,"operation":"execution.status","params":{}}
JSON
```

The `launch_route` field is the answer. It is stamped by the launcher onto the
durable Session at launch, which is the only party that knows.

| Launch mode | Detection | User Verification Handoff |
|---|---|---|
| `autonomous` | `execution.status` reports `launch_route: autonomous` — an unattended gwt Issue Monitor launch | **Waived.** Nobody is watching the session. |
| `interactive` | `launch_route: manual`, or absent | Unchanged: the handoff below runs as written. |

`GWT_AUTONOMOUS_EXECUTION` is a **legacy** signal. Treat it as autonomous when
it is set, but its absence proves nothing: it is written only when the project
opted into unattended autonomous mode, so a monitor-launched window with that
preference off used to read as human-driven and then stall waiting for a human
who was never there (#3777, #3697, #4217). `launch_route` wins over the
environment in every case.

Record the detected mode on the evidence bundle's `Launch mode:` line.

In `autonomous` mode:

- **Skip the User Verification Handoff phase entirely.** Do not prepare a
  user handoff instance, share a verification URL, or call the question tool.
  Prepare an isolated instance for the agent's own headed E2E when UI is in
  scope; for gwt use `browser-check` and the current checkout with a fresh HOME.
- Record `User Verification Result: n/a (autonomous)` for new work, with or
  without a UI surface. This is a launch-mode fact, never `skipped(<reason>)`
  or `confirmed`. The agent's own check does not confirm a human check.
- Require `Agent Visual Check: pass` for UI work and `n/a (no UI surface)`
  otherwise. Automated test, headed E2E, or CI failures and known blockers must
  be repaired; absent human visual confirmation is not a blocker.
- After fresh passing verification and the other Ready PR Gate conditions,
  create a Ready PR or call `pr.ready`, then follow the existing CI auto-merge
  path until merged. Do not stop at a Draft PR or call `execution.blocked`
  merely because no human performed a visual check.

### Legacy deferred results

The legacy `deferred (autonomous execution)` value remains readable on
existing autonomous PRs. Fresh passing verification permits Ready and the
existing CI auto-merge path without rewriting that PR body or requiring human
confirmation. This compatibility does not waive automated evidence or any
other Ready Gate condition. Manual launch verification is unchanged.

`pr.list` with `include: ["body"]` still exposes
`deferred_user_verification`: both `n/a (autonomous)` and the legacy autonomous
deferred value yield `false`; manual or generic deferred results yield `true`.
An absent field means bodies were not hydrated, so the value is unknown.

Record the same result in the tool-generated execution evidence: pass
`params.user_verification_result` to `verify.run` with the exact
`User Verification Result` value used in the report and new PR body. An existing
legacy deferred body may remain unchanged while new autonomous evidence records
`n/a (autonomous)`. The result is persisted in the integrity-protected
Verification Run Record and printed in the run output. For manual launches, an omitted value remains
unknown; it never means `confirmed` or `n/a`. Autonomous launches normalize the
result to `n/a (autonomous)`. Automated command success does not
confirm a human check.

For an autonomous run with a UI surface, for example:

```json
{"schema_version":1,"operation":"verify.run","params":{"commands":["<planned Playwright command>"],"headed_e2e_commands":["<planned Playwright command>"],"user_verification_result":"n/a (autonomous)"}}
```
