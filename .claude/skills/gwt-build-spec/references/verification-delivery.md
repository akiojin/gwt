## Phase 3: Verification

Delegate environment-aware verification to `gwt-verify --mode full`. The
sub-skill is a project-agnostic Generic Verification Contract: it classifies
the changed surfaces against the active runtime's `gwt-verify` skill
(`references/surface-taxonomy.md`),
detects the host project's actual test runners from manifests
(Cargo.toml / package.json / pyproject.toml / go.mod / ProjectSettings /
*.sln / etc.) per `references/runner-detection.md`,
runs the appropriate unit / integration / E2E / visual tests, emits a
**Test Inventory** that names which tests were executed, and provides a
4-step verification path + Check Items only when the owner explicitly requests
a human handoff before finalizing
`## Verification Report`. The right matrix is determined by what actually
changed and which runners the project ships; do not hard-code a static
cargo / pnpm / Playwright command list here.

The skill is considered green when **all** of the following hold:

- `gwt-verify --mode full` exits with `Overall: PASS`
- the evidence bundle records no `failed: tooling-missing` entry
- no visual / UI snapshot diff is unresolved (visual regression must be
  triaged before declaring PASS, not silently regenerated)
- User Verification Result is optional. A human handoff is mandatory only when
  the owner explicitly requests it; a missing, pending, or legacy deferred
  field alone does not create that request. An actual `rejected(<reason>)`
  forces `Overall: FAIL`; preserve the reason and repair the known blocker.
- An autonomous launch is recognized from `execution.status`'s
  `launch_route: autonomous`; record `n/a (autonomous)` and waive the human
  handoff. UI work requires `Agent Visual Check: pass` plus actual passing
  headed Chromium dark/light results in the same fresh `verify.run` record,
  selected with `params.headed_e2e_commands`. Existing autonomous PRs may retain
  the legacy `deferred (autonomous execution)` body value without rewriting it
  or obtaining human confirmation.

In SPEC mode, also verify:

- Implementation matches `spec.md` acceptance scenarios
- No scope creep beyond `tasks.md`
- If a failure indicates a spec gap (not a code bug), route back to `gwt-discussion`

## Phase 4: PR Flow

Handle PR operations autonomously using the `gwt-manage-pr` skill:

- If there is no active PR for the branch, or prior PRs are already merged, use `gwt-manage-pr` to create one.
- If the current PR has CI failures, conflicts, or review blockers, use `gwt-manage-pr` to fix.
- Let `gwt-manage-pr` handle routine merge/push/fix loops.
- In autonomous launches, continue through a Ready PR and the existing
  CI auto-merge path until merged. Absent human visual confirmation does not
  stop delivery; automated test / headed E2E / CI failures, known blockers, and
  other Ready Gate failures still require repair.

Do not create or update a Ready for review PR until Phase 3 verification
passes and the Ready PR Gate confirms a releaseable slice. Draft PRs are
prohibited. Repair failed gates before creating a PR, or select a smaller
releaseable slice with an explicit follow-up boundary. A Draft PR does not satisfy
build completion.

## Phase 5: Completion Gate

### SPEC mode

Reconcile implemented behavior against all SPEC artifacts. See [references/completion-gate.md](completion-gate.md) for details.

Required checks:

- Every completed task in `tasks.md` matches the implementation
- Acceptance checkboxes in the tasks section reflect behavior that has actually been accepted
- TDD checkboxes in the tasks section reflect real verification evidence
- Completion markers in the tasks section do not claim completion that is not backed by code
- Bundled-required owners are covered (SPEC-3248 Multi-Issue Primary Owner
  Bundle): when the Primary owner's tasks/plan classify related Issues/SPECs
  as `bundled-required`, their implementation, tests, and acceptance must be
  complete inside this same execution before reporting done. One primary
  Execution agent/session owns the entire bundled scope end-to-end; helper
  subagents may contribute but never split completion ownership.
  `dependent-follow-up` owners are recorded with explicit ordering and do not
  block Primary completion.
- If artifacts disagree, return to `gwt-discussion` — do not proceed to PR
- Ready PR Gate passed for a releaseable slice. Draft PRs are prohibited and
  do not satisfy completion.

Update execution tracking:

- Mark completed tasks in `tasks.md`
- Update tasks-section checkboxes with JSON operation `issue.spec.edit`

### Standalone mode

- Confirm all user-stated acceptance criteria are met
- Confirm all tests pass
- Confirm lint and type checks pass
- Summarize what was built and what was verified

### Chain suggestion

On completion, suggest `gwt-arch-review` for code review if available, or proceed to `gwt-manage-pr` if not already done.

Only call JSON operation `build.complete` after the Ready PR Gate is
satisfied for a releaseable slice. Report remaining work truthfully; do not
create a Draft PR or represent unfinished work as a completed build.

## Stop conditions

Stop only when:

- the next task depends on a product or scope decision that is not inferable
- a merge conflict or review request is ambiguous enough to risk wrong behavior
- required repo/auth/tooling access is unavailable
- `gwt-verify` returns `failed: tooling-missing` and the auto-install path
  cannot recover (see `gwt-verify`'s `references/tooling-bootstrap.md`)
- implementation reveals a gap between spec.md and the actual behavior
  (acceptance scenario inaccuracy, missing data-model section, undocumented
  registration table, dependency chain not captured in tasks.md, etc.) that
  changes the design surface — not just a typo fix. Route to
  `gwt-discussion` if the gap requires user discussion, or update the
  SPEC artifact directly if the fix is mechanical

## Exit report

```text
## <Build Report in the current user's language>

Mode: SPEC-<id> | Standalone
Completed tasks: <N> | <summary>
Updated files: <paths>
Verification:
- <command/result>

Next:
- `gwt-manage-pr` (create/update PR)
- `gwt-arch-review` (code review)
- return to `gwt-discussion` (artifact repair)
- ask user for decision
```
