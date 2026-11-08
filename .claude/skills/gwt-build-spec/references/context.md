## Mode detection

Determine the mode at entry:

- **SPEC mode** if a SPEC ID is provided or discoverable from the current branch/context, AND
  the tasks section can be read with JSON operation `issue.spec.section`
- **Standalone mode** otherwise

## Phase 1: Context Load

### SPEC mode

1. Establish the approved scope, plan, tasks, and acceptance from current artifacts. Read missing or stale sections with `issue.spec.read` or `issue.spec.section`.
2. Run the Board active-claim preflight for the target SPEC before choosing a
   task slice:
   - Read the current Board with JSON operation `board.show`.
   - Look for active `claim` entries from another session that mention the same
     owner (`#<N>` or `SPEC-<N>`) or the same phase label (`Phase <N>`,
     `Phase <label>`) as the task slice under consideration.
   - If a matching claim exists, pause before Phase 2 and present the conflict:
     propose joining that session with a Board handoff request, splitting the
     work through `gwt-discussion`, or continuing only after the user explicitly
     accepts duplicate risk.
   - Intentional parallel work is allowed only when write scopes are disjoint.
     Post a fresh Board `claim` that includes a `Boundary:` line naming the
     files/modules owned by this session before continuing.
   - Acceptance scenario: Given another session has an active Board claim for
     `SPEC-2008 Phase 24`, when `gwt-build-spec` starts for SPEC-2008, then the
     preflight reports the claim and requires user confirmation before any RED
     test or production edit is made.
3. Identify the next incomplete task slice in dependency order:
   - Setup before Foundational work
   - Foundational before story-specific work
4. Use JSON operations `issue.spec.section` / `issue.spec.edit` for SPEC section reads and writes.

### Standalone mode

1. Accept the user's task description, requirements, and acceptance criteria.
2. Identify target files and test locations from the task description and codebase exploration.
3. No SPEC artifacts are required or created.
