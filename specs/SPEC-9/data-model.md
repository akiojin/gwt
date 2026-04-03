# Data Model: SPEC-9 - Infrastructure

## Primary Entities
### DockerProgressState
- Role: Tracks service setup progress and user-visible Docker status.
- Invariant: Displayed progress must map to real backend lifecycle events.

### EmbeddedSkillManifest
- Role: Describes built-in skill packaging and availability.
- Invariant: Manifest state must stay synchronized with bundled assets.

### HooksMergePlan
- Role: Defines safe merge, backup, and restore handling for git hooks.
- Invariant: Backup and recovery paths must not lose user hooks.

## Lifecycle Notes
- `metadata.json`, `tasks.md`, and `progress.md` must stay aligned.
- Completion cannot be claimed from implementation alone; the checklists must agree.
