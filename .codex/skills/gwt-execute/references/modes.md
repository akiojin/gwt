## Mode detection

1. If the invocation includes `#N` or an Issue URL, read the Issue with JSON
   operations `issue.view`, `issue.comments`, and `issue.linked_prs`.
2. If the Issue has a case-insensitive `gwt-spec` label, use design-gated mode.
3. If the Issue has no `gwt-spec` label, use direct mode.
4. If no owner Issue exists, use standalone mode.

## design-gated mode

Use this for gwt-spec tagged Issues.

1. Read SPEC sections with JSON operations `issue.spec.read` or
   `issue.spec.section`.
2. If `plan` or `tasks` is missing or incomplete, stop before production edits
   and route to `gwt-plan-spec`.
3. Run the Board active-claim preflight for the owner before choosing a task
   slice.
4. Select the next incomplete task in dependency order.
5. Execute strict TDD: write the RED test first, confirm it fails for the
   expected reason, implement the minimum GREEN change, then refactor.
6. Update the owner Issue tasks section through `issue.spec.edit` as tasks are
   completed.

## direct mode

Use this for non-gwt-spec Issues.

1. Establish the issue facts from JSON operations `issue.view`,
   `issue.comments`, and `issue.linked_prs`.
2. For bugs, prove the root cause before editing production code. Do not
   guess-fix.
3. Classify `Spec Status`: `ALIGNED`, `IMPLEMENTATION-GAP`, `SPEC-GAP`, or
   `SPEC-AMBIGUOUS`.
4. If the intended behavior needs design ownership first, route to
   `gwt-discussion` or `gwt-register-issue`; otherwise continue in direct mode.
5. Execute strict TDD with the same Red-Green-Refactor loop.
6. After verification and PR handoff, post the durable closure comment with
   root cause, changed files, verification evidence, PR link, and remaining
   work if any.

## standalone mode

Use this when the user provides an approved implementation task without an
owner Issue.

1. Capture the task description, acceptance criteria, and target files.
2. Identify the narrowest test location from the existing codebase.
3. Execute strict TDD with the same Red-Green-Refactor loop.
4. Do not create Issues or SPECs unless the work proves to need durable design
   ownership.
