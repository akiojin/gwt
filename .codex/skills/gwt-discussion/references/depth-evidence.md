## Depth Interview Loop

Use this loop for every non-trivial discussion, including ordinary
`gwt-discussion` runs. Do not wait for an explicit `--deepen` flag when the
latest answer leaves high-impact ambiguity.

After every answer:

1. Record what was learned in the `Question Ledger`.
2. Re-score unresolved unknowns across scope boundary, ownership/integration,
   failure/edge cases, migration/compatibility, verification/success signal,
   and alternatives/assumption challenge.
3. Ask the next highest-impact question with the platform question tool when
   any category can still change implementation, routing, ownership, or
   verification.
4. Every third question, challenge one premise directly: what breaks if the
   preferred answer is wrong, delayed, partially implemented, or applied to an
   adjacent subsystem?
5. Set `Depth Gate: complete` only when the `Question Ledger` shows all
   applicable categories are covered. Use `Depth Gate: deferred(<reason>)` only
   when the remaining depth is intentionally out of scope for the current
   Action Bundle.

Do not treat a small number of questions as a completion signal. A broad or
risky discussion may require many rounds. Also do not treat a top-3 priority
list as the end of deepening; it is only the next batch to discuss before the
remaining backlog is re-ranked.

## Discussion Depth Gate

Do not stop because one answer landed or because an arbitrary question count
was reached. Re-run this gate after each answer.

Coverage Checks:

- scope boundary
- ownership / integration
- failure / edge case
- migration / compatibility
- verification / success signal

Exit Blockers:

- a high-impact unknown still changes implementation, routing, or ownership
- an applicable coverage category above has not been addressed
- `Open Questions`, `Dependency Checks`, or `Deferred Decisions` still contain
  unresolved high-impact items
- the latest answer introduced a new high-impact unknown

Question depth ladder:

1. Direction / proposal choice
2. Boundary, exceptions, and failure handling
3. Acceptance, verification, and implementation-affecting detail

Keep asking one question at a time until the relevant coverage categories are
closed or explicitly deferred with rationale in `Discussion TODO`.

When the runtime and user allow delegation, a bounded subagent may be used for
objective review of competing proposals. Keep that subagent scoped to
independent comparison work and do not let it replace the main discussion flow.

Do not end the discussion after a single answer when unresolved high-impact
unknowns still exist.

## Evidence Gate

Before producing the final Action Delta / Action Bundle, complete the proof
fields for every `[active]` proposal that will become `[chosen]`.

- `Implementation Proof`: name the implementation files, logs, commands, or
  tests inspected or run. If no implementation exists, say what proves that.
- `SPEC/Issue Proof`: name the owner SPEC / plan / tasks / Issue sections read,
  including any missing or stale items found.
- `Gap Check Proof`: explicitly cover scope, ownership/integration,
  failure/edge cases, migration/compatibility, and verification/success signal.
- `Official Docs Proof`: cite official documentation checked for external API,
  runtime, OS, CLI, library, or hook behavior. Use `not-applicable: <reason>`
  only for purely local behavior.
- `External Research Proof`: cite non-official research, issue trackers, web
  search, or X search when needed. Use `not-applicable: <reason>` only when
  official docs and local evidence fully settle the question.
- `Evidence Gate`: set to `complete` only when the proposal has no unresolved
  `Exit Blockers:` and all proof fields above are non-empty or explicitly
  `not-applicable: <reason>`.

Official documentation is the preferred external evidence. X search and
X API Search Posts official docs are valid discovery paths for fast-moving
facts, but X posts alone are not sufficient for irreversible conclusions unless
they are first-party posts with URL and timestamp, or are corroborated by
official docs or another primary source.
