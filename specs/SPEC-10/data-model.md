# Data Model: SPEC-10 - Project Workspace

## Primary Entities
### WorkspaceBootstrapRequest
- Role: User request to open, clone, or initialize a project workspace.
- Invariant: Bootstrap flow must choose exactly one acquisition path.

### RepositoryKind
- Role: Classification of local repository state such as bare or working tree.
- Invariant: Kind detection must drive migration behavior deterministically.

### MigrationPlan
- Role: Safe conversion steps for adopting older repository layouts.
- Invariant: Migration must preserve repository data before switching modes.

## Lifecycle Notes
- `metadata.json`, `tasks.md`, and `progress.md` must stay aligned.
- Completion cannot be claimed from implementation alone; the checklists must agree.
