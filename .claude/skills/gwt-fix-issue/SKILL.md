---
name: gwt-fix-issue
description: "Use when the user wants to resolve an existing GitHub Issue by number or URL, especially when the workflow should continue through a direct fix unless a SPEC is needed."
---

# gwt-fix-issue

## Transition alias

`gwt-fix-issue` is a temporary alias for `gwt-execute`. If this skill is invoked
with `#N` or an Issue URL, continue as `$gwt-execute #N` and load
the matching `gwt-execute/SKILL.md` asset from the active provider skill tree.
Keep this file only for transition compatibility during the one-release alias
window.

Public task entrypoint for resolving an existing GitHub Issue.

## gwtd resolution

Before executing any `gwtd ...` command from this skill or its references,
resolve `GWT_BIN` first: executable `GWT_BIN_PATH`, then `command -v gwtd`,
then `$GWT_PROJECT_ROOT/target/debug/gwtd` or `./target/debug/gwtd`. Run the
command as `"$GWT_BIN" ...`; if none exists, stop with an actionable
`gwtd not found` error.

## When to use

- The Issue number or URL is already known
- The user says "fix issue #N", "resolve issue", or similar
- The user wants an issue-first workflow without choosing between issue/spec/build internals

Do not use this for new work intake. Use `gwt-register-issue` instead.

## Domain model: an Issue is a bug against a specification

A GitHub Issue handled here is a bug — a deviation from intended behavior. Most
bugs deviate from an owner SPEC that already exists: identify it, prove the
deviation (root cause), and restore conformance. Some bugs fall in a gap that no
SPEC covers; resolving those means settling ownership first — either associating
the behavior with an existing SPEC whose bounded context it belongs to (extend
that SPEC), or authoring a new SPEC when it is a genuinely new context with no
fitting owner. Creating a new SPEC is the last resort, decided through
`gwt-discussion`, never a reflex from this skill.

## Ownership

- Gather the issue facts and related code context
- Identify the owner SPEC the bug deviates from, and prove the root cause
- Decide direct-fix vs spec-needed with a structured `Spec Status`
- Carry direct-fix work through implementation and verification
- Hand off to the visible SPEC flow only when broader design work is required

## Workflow

1. **Verify access.** Confirm JSON operations `issue.view` and `issue.comments`
   can reach the target issue.
2. **Investigate before deciding (mandatory gate).** Gather facts with JSON
   operations `issue.view`, `issue.comments`, `issue.linked_prs`, and the
   bundled `scripts/inspect_issue.py` helper from the current runtime's
   `gwt-fix-issue` skill directory, for example:
   `python3 <this-skill-dir>/scripts/inspect_issue.py --repo "." --issue "<number or URL>"`.
   - For BUG / regression issues, produce the structured report in the bundled
     `references/analysis-report.md` (current runtime's `gwt-fix-issue` skill
     directory) with a **verified root cause** and Evidence-backed ACTIONABLE
     items. If the root cause is unproven, do not implement — establish
     reproduction and root cause first (route to `gwt-discussion` when
     investigation needs user input). Do not guess-fix.
   - For FEATURE / ENHANCEMENT issues, extract requirements and acceptance
     criteria; the full report is optional.
3. **Route with a structured `Spec Status`.** Classify the issue against the
   SPEC space before touching code:
   - `ALIGNED` / `IMPLEMENTATION-GAP` → direct fix. An owner SPEC already
     defines the intended behavior; implementation is missing, broken, or
     incomplete.
   - `SPEC-GAP` (no SPEC covers the behavior) → stop direct work and route to
     `gwt-discussion` to settle ownership first. There, decide by domain fit:
     associate the behavior with an existing SPEC whose bounded context it
     belongs to (extend that SPEC's requirements / acceptance), or author a new
     SPEC only when it is a genuinely new context with no fitting owner. Do not
     create a SPEC as a reflex; the Issue stays the work item either way.
   - `SPEC-AMBIGUOUS` (existing SPECs overlap or conflict) → route to
     `gwt-discussion` to resolve which owner applies before fixing.
4. **Implement through the build/verify loop.** Prefer delegating execution to
   `gwt-build-spec` (Standalone mode) so TDD (Red-Green-Refactor) and
   `gwt-verify` run as designed; its `When to skip tests` rule already exempts
   docs / chore / typo. Verification is mandatory and runs in the correct
   environment:
   - Via `gwt-build-spec`, Phase 3 runs `gwt-verify --mode full` and requires
     `Overall: PASS`.
   - If you fix inline without `gwt-build-spec`, you must still write the
     failing regression test first (TDD) and run `gwt-verify --mode full`
     yourself to `Overall: PASS` before closing. Do not skip TDD or
     verification because the fix looks small; `gwt-verify` self-selects the
     matrix (cargo / frontend / Playwright / docs) for the changed surfaces.
5. **Gate the PR.** PR work goes through `gwt-manage-pr`, which requires
   `gwt-verify --mode pre-pr` and a recorded `User Verification Result`. Do not
   create or update a Ready PR until that result is `confirmed` (UI-affecting
   changes) or `n/a` (no user-visible surface). Never open a Ready PR on a
   `pending` verification. For code-changing direct fixes, PR handoff is part
   of completion: a commit hash, branch push, and closure comment are
   push-only evidence, not completion. If no PR URL or PR number exists yet,
   report `PR handoff pending` or `blocked(<reason>)` and keep working through
   `gwt-manage-pr`; do not say the Issue is complete.
6. **Close with a durable record.** On direct-fix completion, post the mandatory
   closure comment through JSON operation `issue.comment` following the bundled
   `references/closure-comment.md` (current runtime's `gwt-fix-issue` skill
   directory): root cause, changed files, commit/PR link, `gwt-verify` result,
   completion checklist, remaining work — after `gwt-verify` returns
   `Overall: PASS` and `gwt-manage-pr` has produced a PR URL/number or an
   explicit blocked handoff. When the work is handed off to the SPEC flow
   instead of completed, `gwt-build-spec` owns closure; post a short handoff
   comment only.
7. Return the result in the current user's language.

## No Stop-block (short-lived skill)

Per SPEC-1935 FR-014s, `gwt-fix-issue` is intentionally a short-lived skill with
no managed Stop-check hook. This prose is the only lever — there is no runtime
gate forcing continuation — so the investigation, verification, and completion
discipline above is self-enforced. Do not add a Stop-check handler for this
skill.

## Guardrails

- Agent-facing Issue workflow must use gwtd JSON operations `issue.*` as the canonical surface.
- Direct `gh issue ...` commands are not part of the normal path.
- Do not treat the issue body as the source of truth for SPEC artifacts once a SPEC owner exists.
- Do not create both a plain Issue and a SPEC for the same work; keep the existing Issue as the owner.
