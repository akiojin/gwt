# Registration Workflow

## Workflow steps

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

Manage artifact files with the shared helper:

```bash
python3 ".claude/skills/gwt-spec-ops/scripts/spec_artifact.py" \
  --repo "." \
  --spec "<id>" \
  --upsert \
  --artifact "doc:spec.md" \
  --body-file /tmp/spec.md
```

### Create new SPEC

```bash
python3 ".claude/skills/gwt-spec-ops/scripts/spec_artifact.py" \
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

python3 ".claude/skills/gwt-spec-ops/scripts/spec_artifact.py" \
  --repo "." \
  --spec "<id>" \
  --upsert \
  --artifact "doc:spec.md" \
  --body-file /tmp/spec.md
```
