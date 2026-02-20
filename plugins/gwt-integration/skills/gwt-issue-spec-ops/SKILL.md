---
name: gwt-issue-spec-ops
description: GitHub Issue-first SPEC operations for gwt (create/update/sync artifacts and project phase).
---

# gwt Issue SPEC Ops

Use GitHub CLI for issue-first SPEC lifecycle.

## Core operations

- Upsert SPEC issue sections (`spec/plan/tasks/tdd`).
- Upsert/list/delete artifact comments (`contract/checklist`).
- Sync issue to the fixed repository project and update phase.

## Requirements

- `gh` must be installed and authenticated.
- Repository project binding must be configured by gwt.
