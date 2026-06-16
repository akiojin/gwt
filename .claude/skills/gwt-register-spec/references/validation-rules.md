# Validation rules

`gwt-register-spec` validates the SPEC body before any GitHub API call.
This document is the canonical list. The Rust implementation lives at
`crates/gwt-github/src/spec_validate.rs` and exposes
`validate_spec_body(body: &str, title: &str, rules: &ValidationRules) ->
Vec<ValidationIssue>`.

## Severity levels

- `Structural` — the body cannot be processed downstream without manual
  correction. Always hard-fail: no Issue is created.
- `Format` — the body is processable but does not meet the project's
  format convention. Also hard-fail in v1 (consistent registration
  surface). Future opt-in soft-fail is possible behind a `--allow-format`
  flag, but that is out of scope for v1.

Every issue carries `{ severity, location, message }` where `location` is
either `"title"`, the section heading (`"## 機能要件"`), or a 1-based line
offset inside the body.

## Rule table

| ID | Severity | Rule | Failure example |
|---|---|---|---|
| R1 | Structural | Title matches `^SPEC: .+$` | `"feat: foo"` → fail |
| R2 | Structural | First non-blank line of body equals title | H1 says `# something else` |
| R3 | Structural | Section `## 背景` exists | missing or misspelled |
| R4 | Structural | Section `## ユビキタス言語` exists | — |
| R5 | Structural | Section `## ユーザーシナリオと受け入れシナリオ` exists | — |
| R6 | Structural | Section `## 機能要件` exists | — |
| R7 | Structural | Section `## 成功基準` exists | — |
| R8 | Structural | Section `## Out of Scope` exists (case-sensitive prefix; `(v1)` suffix permitted) | `## Out Of Scope` → fail |
| R9 | Structural | Section `## Related Artifacts` exists | — |
| R10 | Structural | `機能要件` section contains ≥1 line matching `^- \*\*FR-\d{3}\*\*` | no FR present |
| R11 | Structural | No occurrence of `[NEEDS CLARIFICATION]` anywhere | unresolved markers |
| R12 | Format | FR identifiers are contiguous (`FR-001`, `FR-002`, …) | `FR-001, FR-003` |

R8 explicitly permits `## Out of Scope (v1)` so version-suffixed sections
keep passing validation. Implementation: prefix match on
`"## Out of Scope"` then optional whitespace + `(...)`.

## Roundtrip check (post-create)

After the GitHub Issue has been created and JSON operation `issue.spec.edit` has
been called, the skill performs a roundtrip verification:

1. JSON operation `issue.spec.section` must exit 0.
2. The output must be non-empty.
3. The output must contain the H1 line of the body (`# SPEC: …`).

If any of these fail, the skill aborts the lifecycle with a
`roundtrip` reason and surfaces the orphan Issue number per
`recovery.md`.

## Section heading detection

Headings are recognised by exact `## <Heading>` at the start of a line
(after optional leading whitespace is rejected — only flush-left
headings count). The match is on the heading text, not on any anchor or
slug. The validator does **not** parse markdown into an AST; it operates
on line patterns to keep the implementation small and predictable.

## What is NOT validated (v1 contract)

- Tense, voice, or style of FR sentences (no NLP).
- Numbering of acceptance scenarios (any 1-based numeric list is
  accepted).
- Existence of plan / tasks sections — those are added later by
  `gwt-plan-spec`.
- Section ordering — sections may appear in any order as long as all
  required headings exist.
- Internal links to other SPECs — `Related Artifacts` may contain any
  free-form text.

## Worked example

Given a body with:

```markdown
# SPEC: Demo

## 背景
ある。

## ユビキタス言語
- **X**: y

## ユーザーシナリオと受け入れシナリオ
### Primary User Story
hi.

## 機能要件
- **FR-001**: a
- **FR-002**: b

## 成功基準
- ok

## Related Artifacts
- none
```

Validation issues returned:

- `Structural` `## ユーザーシナリオと受け入れシナリオ`: missing
  `### Acceptance Scenarios` is not validated (out of scope), but the
  section itself is present → no issue.
- `Structural` `## Out of Scope`: missing → fail.
- (no other issues)

The caller fixes the missing section, re-runs validation, and proceeds.
