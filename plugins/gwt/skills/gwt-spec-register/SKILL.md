---
name: gwt-spec-register
description: Create a new GitHub Issue-first SPEC (`gwt-spec`) using the standard section skeleton when no existing canonical SPEC fits. Use when a plain Issue requires a new SPEC or the user explicitly asks to register a new SPEC.
---

# gwt SPEC Register

Use this skill to create a new `gwt-spec` Issue only after confirming that no existing canonical SPEC should own the scope.

`gwt-spec-register` is a registration step, not an execution step.

- If an existing `gwt-spec` Issue already fits, use `gwt-spec-ops` instead.
- If the user starts from a plain Issue and the correct path is still unclear, use `gwt-issue-resolve` first.

## Mandatory preflight: search existing spec first

Before creating a new spec issue, use `gwt-project-index` Issue search first.

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

## Required issue body structure

New SPEC Issues must use this section structure:

```markdown
<!-- GWT_SPEC_ID:#{number} -->

## Spec

(background, user scenarios, requirements, success criteria)

## Plan

(implementation plan)

## Tasks

(task list)

## TDD

(test design)

## Research

(research notes)

## Data Model

(data model)

## Quickstart

(minimum setup or usage steps)

## Contracts

Artifact files are managed in issue comments with `contract:<name>` entries.

## Checklists

Artifact files are managed in issue comments with `checklist:<name>` entries.

## Acceptance Checklist

- [ ] (acceptance checklist item)
```

## Workflow

1. **Verify gh authentication.**
   - Run `gh auth status` in the repo.
   - If unauthenticated, stop and ask the user to log in.

2. **Search for an existing canonical SPEC.**
   - Use `gwt-project-index` Issue search with at least 2 queries.
   - If a canonical SPEC exists, stop registration and hand off to `gwt-spec-ops`.

3. **Create the new `gwt-spec` Issue.**
   - Use the built-in spec issue creation path when available.
   - If the built-in path is unavailable, use the documented `gh issue create` fallback.
   - After creation, update `<!-- GWT_SPEC_ID:#NEW -->` to `<!-- GWT_SPEC_ID:#{number} -->`.

4. **Seed the initial sections.**
   - Fill `## Spec` with the minimum context from the originating Issue or request.
   - Create stub `## Plan`, `## Tasks`, and `## TDD` sections so execution can continue without inventing a second format.
   - Leave unresolved details explicit instead of hiding them.

5. **Hand off to `gwt-spec-ops`.**
   - Pass the created issue number and source context into `gwt-spec-ops`.
   - `gwt-spec-register` should not own long-running SPEC execution.

## Operations (gh CLI fallback)

### Create new spec issue

```bash
gh issue create --label gwt-spec --title "feat: ..." --body "$(cat <<'EOF'
<!-- GWT_SPEC_ID:#NEW -->

## Spec

_TODO_

## Plan

_TODO_

## Tasks

_TODO_

## TDD

_TODO_

## Research

_TODO_

## Data Model

_TODO_

## Quickstart

_TODO_

## Contracts

Artifact files are managed in issue comments with `contract:<name>` entries.

## Checklists

Artifact files are managed in issue comments with `checklist:<name>` entries.

## Acceptance Checklist

- [ ] Add acceptance checklist
EOF
)"
```

### Finalize marker after creation

```bash
gh issue edit {number} --body "$(updated body with <!-- GWT_SPEC_ID:#{number} -->)"
```
