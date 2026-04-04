# PR Fix Policies and Formatting Rules

## Comment Response Policy

> **No reviewer comment may be left unanswered.**

- Every unresolved review thread MUST receive a reply before being resolved.
- If the feedback was addressed: reply with what was done (e.g., "Fixed: refactored as suggested.").
- If the feedback was intentionally not addressed: reply with the reason (e.g., "Not addressed: this is intentional because the API contract requires this format.").
- The `--reply-and-resolve` argument enforces this by requiring a reply entry for every unresolved thread and rejecting empty bodies.

## Diagnosis Report Anti-Patterns

### Prohibited Language

| Prohibited | Required Alternative |
|---|---|
| "We should look into..." | "Edit `path/file.ts:42` to..." |
| "There seem to be some issues" | "3 blocking items detected" |
| "This might be causing..." | "Root cause: `<error from log>`" |
| "Consider fixing..." / "It looks like..." | "Action: Fix `<what>` in `<where>`" |
| "Various CI checks are failing" | "2 CI checks failing: `build`, `lint`" |
| "Some reviewers have concerns" | "@reviewer1 requested: `<quote>`" |
| "I'll try to fix this" | "Action: \<specific fix\>" |

### Structural Prohibitions

- Prose paragraphs for reporting — use B1/I1 item format exclusively.
- Omitting the Evidence field in any BLOCKING item.
- Combining multiple independent problems into a single item.
- Omitting file paths or line numbers when the script output contains them.

## Issue/PR Comment Formatting

- Final comment text must not contain escaped newline literals such as `\n`.
- Use real line breaks in comment bodies. Do not rely on escaped sequences for formatting.
- Before posting (`--add-comment` or manual `gh issue/pr comment`), verify the final body does not accidentally include escaped control sequences (`\n`, `\t`).
- If a raw escape sequence must be shown for explanation, include it only inside a fenced code block and clarify it is intentional.

## Issue Progress Comment Template

When work is tracked in GitHub Issues, progress updates must use this template:

```markdown
Progress
- ...

Done
- ...

Next
- ...
```

- Post updates at least when starting work, after meaningful progress, and when blocked/unblocked.
- In `Next`, explicitly state blockers or the immediate next action.
