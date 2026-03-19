---
name: gwt-spec-register
description: Create a new GitHub Issue-first SPEC container when no existing canonical SPEC fits. Seed the Issue body as an artifact index plus a `spec.md` comment, then continue into SPEC orchestration unless the user explicitly asks for register-only behavior.
---

# gwt SPEC Register

Use this skill to create a new `gwt-spec` Issue container only after confirming that no
existing canonical SPEC should own the scope.

`gwt-spec-register` owns creation, then normally returns control to `gwt-spec-ops`.

- If the user wants to register new work and it is still unclear whether it should become a plain Issue or a SPEC, use `gwt-issue-register` first.
- If an existing `gwt-spec` Issue already fits, use `gwt-spec-ops` instead.
- If the user starts from an existing plain Issue and the correct path is still unclear, use `gwt-issue-resolve` first.

## Mandatory preflight: search existing spec first

Before creating a new spec issue, use `gwt-issue-search` first.

Required behavior:

1. Ensure `gh auth status` is valid before any `index-issues` call
2. Update the Issues index if needed
3. Run semantic Issue search with queries derived from the current request
4. Prefer an existing canonical integrated SPEC over creating a new point SPEC
5. Create a new `gwt-spec` Issue only when no suitable canonical SPEC exists

Do not create a new SPEC when an existing canonical SPEC clearly owns the scope.

## SPEC ID and label

- SPEC ID = GitHub issue number
- Spec issues must carry the `gwt-spec` label

## Required Issue body structure

New SPEC Issues must use this compact index structure:

```markdown
<!-- GWT_SPEC_ID:#{number} -->

## Artifact Index

- `doc:spec.md`
- `doc:plan.md` (planned)
- `doc:tasks.md` (planned)
- `doc:research.md` (planned)
- `doc:data-model.md` (planned)
- `doc:quickstart.md` (planned)
- `contract:*`
- `checklist:*`

## Status

- Phase: Specify
- Clarification: Required
- Planning: Pending `gwt-spec-ops`

## Links

- Parent: ...
- Related: ...
- PRs: ...
```

## Required initial `spec.md` artifact

After issue creation, create a `doc:spec.md` issue comment with this minimum structure:

```markdown
<!-- GWT_SPEC_ARTIFACT:doc:spec.md -->
doc:spec.md

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

1. **Verify gh authentication.**
   - Run `gh auth status` in the repo.
   - If unauthenticated, stop and ask the user to log in.

2. **Search for an existing canonical SPEC.**
   - Use `gwt-issue-search` with at least 2 queries.
   - If a canonical SPEC exists, switch to `gwt-spec-ops` and continue there.

3. **Create the new `gwt-spec` Issue.**
   - Use the built-in spec issue creation path when available.
   - If the built-in path is unavailable, use the documented `gh issue create` fallback.
   - If `gh issue create` / `gh issue edit` hits a secondary rate limit, use the REST issue endpoints (`POST` / `PATCH /repos/<owner>/<repo>/issues/...`) instead of stopping.
   - After creation, update `<!-- GWT_SPEC_ID:#NEW -->` to `<!-- GWT_SPEC_ID:#{number} -->`.

4. **Seed the initial `spec.md` artifact.**
   - Fill the artifact with the minimum context from the originating Issue or request.
   - Use `[NEEDS CLARIFICATION: ...]` instead of guessing.
   - Do not create `plan.md` or `tasks.md` here.

5. **Continue through `gwt-spec-ops` unless register-only was explicitly requested.**
   - Pass the created issue number and source context into `gwt-spec-ops`.
   - `gwt-spec-register` should not stop at the first handoff boundary when the user's request is to keep moving.

## Operations (gh CLI fallback)

Artifact comments should be created with the shared helper:

```bash
python3 "${CLAUDE_PLUGIN_ROOT}/skills/gwt-spec-ops/scripts/spec_artifact.py" \
  --repo "." \
  --issue "<number>" \
  --upsert \
  --artifact "doc:spec.md" \
  --body-file /tmp/spec.md
```

The shared helper uses GitHub issue comment REST endpoints for create/update and should be preferred over raw `gh issue comment`.

### Create new spec issue

```bash
gh issue create --label gwt-spec --title "feat: ..." --body "$(cat <<'EOF'
<!-- GWT_SPEC_ID:#NEW -->

## Artifact Index

- `doc:spec.md`
- `doc:plan.md` (planned)
- `doc:tasks.md` (planned)
- `doc:research.md` (planned)
- `doc:data-model.md` (planned)
- `doc:quickstart.md` (planned)
- `contract:*`
- `checklist:*`

## Status

- Phase: Specify
- Clarification: Required
- Planning: Pending `gwt-spec-ops`

## Links

- Parent: ...
- Related: ...
- PRs: ...
EOF
)"
```

### Finalize marker after creation

```bash
gh issue edit {number} --body "$(updated body with <!-- GWT_SPEC_ID:#{number} -->)"
```

If `gh issue create` or `gh issue edit` is rate-limited, fall back to:

```bash
gh api "repos/<owner>/<repo>/issues" --method POST --input /tmp/spec-create.json
gh api "repos/<owner>/<repo>/issues/{number}" --method PATCH --input /tmp/spec-edit.json
```

### Create initial `spec.md` artifact comment

```bash
cat <<'EOF' >/tmp/spec.md
<!-- GWT_SPEC_ARTIFACT:doc:spec.md -->

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
  --issue "{number}" \
  --upsert \
  --artifact "doc:spec.md" \
  --body-file /tmp/spec.md
```
