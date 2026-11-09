## Verification and PR gate

Phase 3 delegates to `gwt-verify --mode full`. Record the selected commands and
results in the evidence bundle. For a releaseable slice in a project with an approved
CI-backed policy, use `gwt-verify --mode pre-pr` instead: keep the Issue AC tests
and checks absent from required CI locally, then wait for required CI before
claiming delivery. Full verification remains available and is the fallback
when the CI correspondence cannot be established. User Verification Result is optional
in every launch mode. A human handoff is mandatory only when the owner explicitly requests
it; a missing, pending, or legacy deferred field alone does not create that request.

Read the launch route from the launch record, not from the environment:
`execution.status` reports `launch_route: autonomous | manual`. The legacy
`GWT_AUTONOMOUS_EXECUTION` marker still means autonomous when present, but its
absence proves nothing — it is written only when the project opted into
unattended mode, so monitor launches used to misread themselves as human-driven
and stall (#3777, #3697, #4217).

In an `autonomous` launch, skip the user handoff and record
`User Verification Result: n/a (autonomous)`. Do not ask for visual confirmation
or send a verification URL. Cover any UI surface with the agent's own automated
headed E2E, recorded separately as `Agent Visual Check: pass` (`n/a (no UI
surface)` otherwise). Never turn the agent's check into human `confirmed`.

Run the full matrix through `verify.run`. For UI work, select its Playwright
commands with `params.headed_e2e_commands`; each entry must exactly match an
entry in `params.commands`. gwtd adds `--headed` and its embedded reporter and
records actual Chromium results for both dark and light themes. Use
`browser-check` for an isolated checkout instance; the E2E tests must assert the
changed behavior and zero console/page errors. An autonomous UI Ready handoff
requires those measured passing results in the same fresh verification record.

Before Ready PR creation or enabling auto-merge, compare the owner Issue's
acceptance criteria with the fresh `verify.run` evidence. Update only proven
AC checkboxes through `issue.edit` (`issue.spec.edit` for a `gwt-spec` owner),
then read the owner back to confirm the saved state. Leave unproven criteria
unchecked, including delivery criteria until `origin/develop` contains the
source. Keep the same launch active through merge and post-merge settlement;
declare delivery complete only after confirming `origin/develop` contains the
source.

PR work goes through `gwt-manage-pr`. After automated verification and all
other Ready PR Gate conditions pass, create a Ready PR (or call `pr.ready` for
an existing Draft), then follow the existing CI auto-merge path until the PR is
merged. Draft PRs are prohibited. Automated test / headed E2E / CI failures,
known blockers, and other Ready Gate failures still require repair.

The legacy `deferred (autonomous execution)` value remains compatible on
existing autonomous PRs: fresh passing evidence permits Ready without rewriting
the PR body or obtaining human confirmation. Any actual rejection still
records a known blocker that must be repaired.

**Never call `execution.blocked` merely because an autonomous launch has no
human visual confirmation.** It is a terminal outcome, not a pause. Continue
through verification, Ready PR, and CI auto-merge; settle the execution after
the scoped delivery is complete.
