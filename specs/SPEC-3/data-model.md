# Data Model: SPEC-3 - Agent Management

## Primary Entities
### AgentDefinition
- Role: Describes a built-in or custom agent that can be launched from the wizard.
- Invariant: Definitions must remain valid across cache refreshes and session conversion.

### VersionCache
- Role: Stores discovered version information for launch-time decisions.
- Invariant: Refresh scheduling must not block startup flow.

### SessionConversionRequest
- Role: Carries the target agent and current session context during conversion.
- Invariant: Repo path and session identity must survive a conversion attempt.

## Lifecycle Notes
- `metadata.json`, `tasks.md`, and `progress.md` must stay aligned.
- Completion cannot be claimed from implementation alone; the checklists must agree.
