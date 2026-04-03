---
name: gwt-spec-brainstorm
description: "Use when a user starts with a rough idea, title-level request, or pre-SPEC brainstorming request and the workflow must decide whether the work belongs in an existing SPEC, a new SPEC, or a plain Issue before drafting artifacts."
---

# gwt SPEC Brainstorm

Use this skill as the pre-SPEC intake entrypoint for rough requests.

`gwt-spec-brainstorm` stays in pre-SPEC space.

- Do not create `spec.md` at the start.
- Do not jump directly to `gwt-spec-register` or `gwt-issue-register` before search and interview.
- If the user already has a SPEC ID, use `gwt-spec-ops` instead.
- If the user already has an Issue number or URL, use `gwt-issue-resolve` instead.

## Goals

- Turn a rough request into a decision-ready intake.
- Reuse an existing owner when one already exists.
- Ask only the highest-impact questions needed to route correctly.
- Hand off automatically to the owning workflow once the path is clear.

## Mandatory preflight: search before inventing

Before deciding whether to create or update anything:

1. Use `gwt-issue-search` with at least 2 semantic queries derived from the request.
2. Use `gwt-spec-search` with at least 2 semantic queries derived from the request.
3. If ownership is still unclear, also inspect local `specs/` via `spec_artifact.py --repo . --list-all`.
4. Prefer an existing canonical SPEC or Issue when a clear owner exists.
5. Only continue toward new work when the duplicate check is clean or the remaining ambiguity is a true product decision.

Do not skip search because the idea "sounds new".

## Interview mode

The interview is mandatory.

- Use the platform-native question tool when available. Examples include Claude Code's question tool or the equivalent Codex question UI.
- If no such tool is available, ask one concise question directly in chat.
- Ask exactly one question at a time and wait for the answer.
- Prefer multiple-choice framing when it reduces ambiguity quickly.
- Ask only what materially changes routing, scope, behavior, or testability.
- Do not ask questions already answered by the repo, existing specs, or duplicate search.
- Do not answer product or scope decisions on the user's behalf.
- If the request is too broad for one SPEC, stop and decompose it before registration.

Prioritize questions in this order:

1. user goal / why now
2. target user or actor
3. current pain or missing capability
4. scope boundary and explicit non-goals
5. success condition for the first slice
6. integration target or owning subsystem

## Intake memo

Maintain a short working note during the interview.
Keep it in the conversation or scratch context; do not turn it into `spec.md` yet.

Use this structure:

```markdown
## Intake Memo

- Request:
- Why now:
- Users / actors:
- In scope:
- Out of scope:
- Constraints:
- Success signal:
- Open questions:
- Candidate owner:
```

Keep the memo short and update it as answers arrive.

## Decision outcomes

After enough signal is gathered, produce this summary:

````text
## Registration Decision

**Duplicate Check:** CLEAN | EXISTING-ISSUE | EXISTING-SPEC | AMBIGUOUS
**Chosen Path:** EXISTING-SPEC | NEW-SPEC | ISSUE | TOO-BROAD-SPLIT-FIRST
**Action:** <specific next step>

### Intake Memo
- <final memo bullets>

### Candidates
- #<number> <title> [issue] - <why it matches>
- SPEC-<id> <title> [spec] - <why it matches>
````

## Handoff rules

Once the path is clear, continue automatically:

- `EXISTING-SPEC` -> `gwt-spec-ops`
- `NEW-SPEC` -> `gwt-spec-register`, then `gwt-spec-ops`
- `ISSUE` -> `gwt-issue-register`
- `TOO-BROAD-SPLIT-FIRST` -> ask the user to choose the first slice, then continue with this skill

Do not ask "should I proceed?" after the decision. Move into the next skill unless a real product decision still blocks progress.

## Exit criteria

Do not hand off until all of the following are true:

- duplicate / owner search has been run
- the user's goal is clear enough to route correctly
- the first slice has a visible success condition
- high-impact scope ambiguity has been resolved by the user
- the SPEC vs Issue decision is explicit and justified

## Typical triggers

- "こんなことをしたい"
- "SPEC にする前に壁打ちしたい"
- "表題だけ決まっていて中身を詰めたい"
- "既存 SPEC に統合すべきか見て"
- "これって Issue で十分?"
- "新機能の方向性を整理したい"
