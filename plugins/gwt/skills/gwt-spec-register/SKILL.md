---
name: gwt-spec-register
description: Create a new local SPEC directory when no existing canonical SPEC fits. Create `specs/SPEC-{id}/` with `metadata.json` + `spec.md`, then continue into SPEC orchestration unless the user explicitly asks for register-only behavior.
---

# gwt SPEC Register

Use this skill to create a new local SPEC directory only after confirming that no
existing canonical SPEC should own the scope.

`gwt-spec-register` owns creation, then normally returns control to `gwt-spec-ops`.

- If the user wants to register new work and it is still unclear whether it should become a plain Issue or a SPEC, use `gwt-issue-register` first.
- If an existing SPEC already fits, use `gwt-spec-ops` instead.
- If the user starts from an existing plain Issue and the correct path is still unclear, use `gwt-issue-resolve` first.

## Mandatory preflight: search existing spec first

Before creating a new SPEC, use `gwt-issue-search` first.

Required behavior:

1. Update the Issues index if needed
2. Run semantic search with queries derived from the current request
3. Also search local `specs/` directory for existing SPECs via `spec_artifact.py --repo . --list-all`
4. Prefer an existing canonical integrated SPEC over creating a new point SPEC
5. Create a new SPEC only when no suitable canonical SPEC exists

Do not create a new SPEC when an existing canonical SPEC clearly owns the scope.

## SPEC ID and directory

- SPEC ID = sequential number (e.g., `SPEC-1776`)
- SPECs are stored as local directories under `specs/SPEC-{id}/`

## Title convention

All SPECs must use the `gwt-spec:` prefix:

```text
gwt-spec: <concise English description>
```

- Always use the `gwt-spec:` prefix (not `機能仕様:`, `バグ修正仕様:`, `feat:`, etc.)
- The description should be a short imperative summary in English

## SPEC directory structure

New SPECs are created as local directories:

```text
specs/SPEC-{id}/
  metadata.json      # {"id","title","status","phase","created_at","updated_at"}
  spec.md
  plan.md            (created later)
  tasks.md           (created later)
  research.md        (created later)
  data-model.md      (created later)
  quickstart.md      (created later)
  contracts/
  checklists/
```

### `metadata.json` structure

```json
{
  "id": "SPEC-{id}",
  "title": "gwt-spec: <description>",
  "status": "open",
  "phase": "Specify",
  "created_at": "2024-01-01T00:00:00Z",
  "updated_at": "2024-01-01T00:00:00Z"
}
```

## Required initial `spec.md` artifact

After directory creation, create `specs/SPEC-{id}/spec.md` with this minimum structure:

```markdown
# Feature Specification: <title>

## Background

...

## User Stories

### User Story 1 - <title> (Priority: P1)

...

## Acceptance Scenarios

1. Given ...

## Edge Cases

- ...

## Functional Requirements

- FR-001 ...

## Non-Functional Requirements

- NFR-001 ...

## Success Criteria

- SC-001 ...
```

## Workflow

1. **Search for an existing canonical SPEC.**
   - Use `gwt-issue-search` with at least 2 queries.
   - Also search local `specs/` via `spec_artifact.py --repo . --list-all`.
   - If a canonical SPEC exists, switch to `gwt-spec-ops` and continue there.

2. **Create the new SPEC directory.**
   - Use the built-in spec creation command.

3. **Seed the initial `spec.md` artifact.**
   - Fill the artifact with the minimum context from the originating Issue or request.
   - Use `[NEEDS CLARIFICATION: ...]` instead of guessing.
   - Do not create `plan.md` or `tasks.md` here.

4. **Continue through `gwt-spec-ops` unless register-only was explicitly requested.**
   - Pass the created SPEC ID and source context into `gwt-spec-ops`.
   - `gwt-spec-register` should not stop at the first handoff boundary when the user's request is to keep moving.

## Operations

Artifact files should be managed with the shared helper:

```bash
python3 "${CLAUDE_PLUGIN_ROOT}/skills/gwt-spec-ops/scripts/spec_artifact.py" \
  --repo "." \
  --spec "<id>" \
  --upsert \
  --artifact "doc:spec.md" \
  --body-file /tmp/spec.md
```

### Create new SPEC

```bash
python3 "${CLAUDE_PLUGIN_ROOT}/skills/gwt-spec-ops/scripts/spec_artifact.py" \
  --repo "." \
  --create \
  --title "gwt-spec: ..."
```

### Create initial `spec.md` artifact

```bash
cat <<'EOF' >/tmp/spec.md
# Feature Specification: ...

## Background

...

## User Stories

### User Story 1 - ... (Priority: P1)

...

## Acceptance Scenarios

1. Given ...

## Edge Cases

- ...

## Functional Requirements

- FR-001 ...

## Non-Functional Requirements

- NFR-001 ...

## Success Criteria

- SC-001 ...
EOF

python3 "${CLAUDE_PLUGIN_ROOT}/skills/gwt-spec-ops/scripts/spec_artifact.py" \
  --repo "." \
  --spec "<id>" \
  --upsert \
  --artifact "doc:spec.md" \
  --body-file /tmp/spec.md
```
