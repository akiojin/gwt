# Closure Comment Template

Use this template for the closure comment posted on an Issue when a direct fix
is completed through `gwt-fix-issue`. The closure comment is the durable record
of what was fixed and how it was verified, so the issue history stays useful
long after the conversation is gone.

## When to use

- Post the closure comment on **direct-fix completion**, after `gwt-verify
  --mode full` returns `Overall: PASS` and PR handoff has produced a PR URL or
  PR number for code-changing work.
- A pushed branch without a PR is push-only evidence, not completion. In that
  state, report `PR handoff pending` or `blocked(<reason>)` instead of posting a
  completion closure.
- Do **not** use this template when the work is handed off to the SPEC flow
  instead of completed. In that case `gwt-build-spec` owns closure; post a short
  handoff comment only.

## Required fields

Every closure comment must fill all six fields. Do not omit a field; if a field
is genuinely empty, state that explicitly (e.g. `Remaining Work: none`).

### Root Cause

The verified reason the issue occurred, stated as fact. Name the responsible
code path or behavior. Do not speculate ("might be", "seems like"); if the cause
is unproven, the fix is not ready to close.

### Changed Files

The concrete files touched, each with a one-line note on what changed. Use
`path/to/file:line` form when a specific location matters.

### Commit and PR

The commit hash(es) and PR URL/number that deliver the fix. For code-changing
direct fixes, commit-only or push-only is not completion.

### Verification

The exact `gwt-verify --mode full` outcome (`Overall: PASS`) plus the runners it
selected for the changed surfaces (cargo / frontend / Playwright / docs). Name
any test or command run beyond the matrix.

### Completion Checklist

The pre-completion self-check, mirroring the project's done criteria. Mark each
item explicitly; if one does not apply, say so. Cover at least: all relevant
tests pass, lint / type checks pass, no leftover TODO or unfinished work, the
owner SPEC / Issue is updated when applicable, and the change is committed and
pushed.

### Remaining Work

Any follow-up that is intentionally out of scope, or `none`. Remaining items
must not be delivery blockers for this fix.

## Comment template

```text
## Fix Summary: #<number>

**Root Cause:** <verified cause>

**Changed Files:**
- `path/to/file.ext` — <what changed>

**Commit / PR:** <hash and PR URL/number>

**Verification:** `gwt-verify --mode full` → Overall: PASS
- Runners: <cargo | frontend | Playwright | docs>
- Additional: <any extra command/test, or none>

**Completion Checklist:**
- [ ] All relevant tests pass
- [ ] Lint / type checks pass
- [ ] No leftover TODO or unfinished work
- [ ] Owner SPEC / Issue updated if applicable
- [ ] Committed and pushed

**Remaining Work:** <follow-up items, or none>
```

## Posting

Write the comment body to a file and post it with the canonical CLI:

```bash
body_json="$(jq -Rs . < /tmp/issue-comment.md)"
"$GWT_BIN" <<JSON
{"schema_version":1,"operation":"issue.comment","params":{"number":123,"body":$body_json}}
JSON
```

Direct `gh issue comment ...` is not part of the normal path.

## Anti-patterns

- Omitting any of the six required fields.
- Speculative root cause ("this might be caused by ...") instead of a verified
  statement.
- Claiming completion without the `gwt-verify --mode full` `Overall: PASS`
  result in the Verification field.
- Claiming completion for code-changing work with only a commit hash, pushed
  branch, or closure comment and no PR URL/number.
- Listing "various files" or "some changes" instead of concrete paths.
- Using this template for a SPEC handoff instead of a completed direct fix.
