## Flow

### Phase 1: Theme + search

1. Understand the user's concern, idea, or implementation gap.
2. Use the `gwt-search` skill (a skill, not a PATH command) with 2-3
   semantic queries in Japanese and English.
3. Check open SPEC Issues with JSON operation `issue.spec.list`.
4. If an existing Issue number or URL is already the primary owner, capture it
   in the `Intake Memo`.
5. If a clear owner exists, present it before going further.
6. If the clear owner is an existing SPEC, run the Board active-claim preflight
   before changing the work-notes discussion log or any SPEC/plan artifacts:
   - Read the current Board with JSON operation `board.show`.
   - Look for active `claim` entries from another session that mention the same
     owner (`#<N>` or `SPEC-<N>`) or the same phase/topic under discussion.
   - If a matching claim exists, pause the discussion flow and present the
     conflict: join that session with a Board handoff request, redesign a
     disjoint work split, or continue only after the user explicitly accepts
     duplicate risk.
   - Intentional parallel discussion/planning is allowed only when ownership is
     disjoint. Post a fresh Board `claim` with a `Boundary:` line naming the
     topic, artifacts, or files owned by this session before continuing.
   - Acceptance scenario: Given another session has an active Board claim for
     `SPEC-2008 Phase 24`, when `gwt-discussion --deepen SPEC-2008` starts,
     then the preflight reports the claim and requires user confirmation before
     producing an Action Delta or changing SPEC artifacts.

### Phase 2: Investigation

Do not ask the user anything until you have investigated.

- Grep for related functions, types, constants, and commands
- Read the files or artifacts that would change
- If possible, run the code or command to observe actual behavior
- Compare code, SPEC, plan, and issue state when they disagree
- Check official documentation for external APIs, runtimes, operating systems,
  CLIs, libraries, hooks, or platform behavior that cannot be proven locally
- Use web research when repo-local evidence and official documentation are not
  enough. For fast-moving operational facts, use X search when available, and
  record the source account, URL, timestamp, and whether it is first-party

For every potential change surface, map:

```markdown
| Change | Downstream impact | Upstream prerequisite | Must change together |
|---|---|---|---|
| ... | ... | ... | ... |
```

### Phase 3: Discuss

Present findings first:

- expected vs actual behavior
- dependency chains
- gaps between SPEC, plan, issue, and code
- which proposal is currently `active`, `parked`, `rejected`, or `chosen`

Ask the user one question at a time about the findings.
Wait for their answer before asking the next question.

Use an available question tool when an answer would resolve a consequential unknown; ordinary conversation is also suitable. Each question should:

- use a selection UI when available
- offer 2-3 mutually exclusive options
- put the recommended option first
- avoid freeform text unless options would distort the decision

After each answer:

- update the `Intake Memo`
- update the `Discussion TODO`
- append or refine the active proposal's `Question Ledger`
- update `Depth Gate` only when depth coverage is complete or explicitly
  deferred with a reason
- remove or refine the resolved unknown
- re-rank the remaining unresolved questions
- Ask the next highest-impact question if any remain
