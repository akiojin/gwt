# SPEC body template

The 8 required sections, in canonical order. Copy this skeleton, fill in
each section, and pass the resulting file to `gwt-register-spec` via
`body_path`.

The H1 line MUST equal the GitHub Issue title (the same `SPEC: <title>` you
pass as `title`). See `validation-rules.md` for the full rule set.

```markdown
# SPEC: <one-line work name>

## 背景

<Why this work is needed. What problem or constraint prompted it. Reference
prior incidents, memory, or upstream decisions. Keep to 3-6 paragraphs.>

## ユビキタス言語

- **<TermA>**: <one-sentence definition that the team agrees on>
- **<TermB>**: <one-sentence definition>
- <Add as many entries as needed; this section is the shared vocabulary for
  the rest of the SPEC. Reviewers should be able to read the rest of the
  SPEC and know exactly what each named concept refers to.>

## ユーザーシナリオと受け入れシナリオ

### Primary User Story

<One paragraph: which user, what outcome, why it matters.>

### Acceptance Scenarios

1. **<short name>**: <Given / When / Then narrative. Be concrete enough to
   write a test against it.>
2. **<short name>**: <...>
3. **<short name>**: <...>

## 機能要件

- **FR-001**: <First requirement. Use the FR-NNN pattern with contiguous
  numbers. Each requirement is one declarative sentence the implementation
  must satisfy.>
- **FR-002**: <...>
- **FR-003**: <...>
<Add as many FRs as needed. The numbering must be contiguous (FR-001,
FR-002, …) — gaps cause a Format validation issue.>

## 成功基準

- <Concrete verification command or observable outcome. Prefer command +
  expected result over prose.>
- <Example: `cargo test -p gwt-github spec_validate::` が GREEN.>
- <Example: `pnpm lint:skills` が新 SKILL.md frontmatter で通過.>

## 受け入れ基準

- [ ] AC-1: <One machine-checkable criterion. Keep the exact `- [ ] AC-N:`
      prefix; the Issue Monitor's autonomous gate only reads this shape.>
- [ ] AC-2: <...>
- [ ] AC-3: <Append `(visual)` when a human must look at a screen.>

## Out of Scope (v1)

- <Explicitly excluded behaviors. Use this to prevent scope creep during
  implementation.>
- <Items that belong in v2 or in a different SPEC.>

## Related Artifacts

- Plan: `<path to plan file if any>`
- Discussion state: `.gwt/discussion.md` (Proposal X [chosen] / Evidence
  Gate complete)
- Reference: `<related SPEC, docs, code path>`
```

## Section-by-section guidance

- **背景** — answer "why now". Forward-looking only; do not repeat the
  ユビキタス言語 definitions here.
- **ユビキタス言語** — required even for small SPECs. If the work introduces
  no new terms, list the load-bearing existing terms so reviewers do not
  have to guess.
- **ユーザーシナリオと受け入れシナリオ** — `Primary User Story` is one
  paragraph; `Acceptance Scenarios` is a numbered list. Each scenario
  should be implementable as a black-box test.
- **機能要件** — declarative `FR-NNN` lines. Imperative voice ("must …") is
  fine. Avoid "should" language; ambiguity blocks downstream planning.
- **成功基準** — prefer commands and observable signals. Subjective bullets
  ("ユーザー体験が向上する") are not measurable and should be deleted. This
  section is prose for humans: the verification command and its expected
  result. It is **not** read by the Issue Monitor.
- **受け入れ基準** — the machine-checkable checklist the Issue Monitor's
  autonomous gate (`classify_acceptance_criteria`) reads. Every line must be
  `- [ ] AC-N: <text>` directly under this exact heading (`## Acceptance
  Criteria` and `## 受け入れ条件` are also accepted; `## 成功基準` is not).
  A `- [ ]` line without the prefix is numbered by position, but keep the
  explicit `AC-N:` so ids stay stable across edits. Do not mix the two
  styles in one block: when any item carries an explicit `AC-N:`, the
  un-prefixed lines in that block are dropped from the criteria. The
  classifier reads
  this section wherever the storage layer put it (body or comment), so a
  comment-resident `spec` section is fine. An `auto-merge` Issue
  without this block is refused at `issue.create` / `issue.spec.create` /
  `issue.spec.edit` time and, if it slips through, lands in `needs_human`.
  Keep 成功基準 (how to verify) and 受け入れ基準 (what must be true, one
  checkbox each) separate; do not put `AC-N` lines under 成功基準.
- **Out of Scope (v1)** — required even if empty (write `- なし` if there
  are no deferred items). Forces the author to think about scope.
- **Related Artifacts** — paths and links only. No prose summaries here;
  the SPEC body itself is the canonical narrative.

## Common mistakes

- Listing `FR-001` and `FR-003` but no `FR-002` — Format validation fails.
- Adding `[NEEDS CLARIFICATION]` markers and forgetting to resolve them
  before calling `gwt-register-spec`. Use `gwt-discussion` to resolve them
  first.
- Using `**Functional Requirements**` or English-only section headings
  instead of the canonical Japanese names. The validator looks for exact
  Japanese headings.
- Placing `- [ ] AC-N:` lines under `## 成功基準` instead of `## 受け入れ基準`.
  The Monitor never scans 成功基準, so the SPEC is not autonomy-eligible
  (Issue #3873). If it already happened, fix the body and run
  `issue.monitor.requeue`; a body edit alone is not re-evaluated.
- Forgetting to keep H1 in sync with the GitHub Issue title argument.
