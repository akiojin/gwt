# Registration Reference

Detailed logic for Phase 3 of gwt-design.

## Prerequisites

Phase 3 runs only when Phase 1 routing produced `NEW-SPEC` and Phase 2 domain
discovery confirmed the scope is right for a SPEC.

## SPEC ID allocation

SPEC IDs are sequential integers. Determine the next ID by listing existing SPECs:

```bash
python3 ".claude/scripts/spec_artifact.py" --repo "." --list-all
```

Pick the next available number.

## Title convention

All SPECs must use the `gwt-spec:` prefix:

```text
gwt-spec: <concise English imperative description>
```

- Always use `gwt-spec:` (not other prefixes).
- Description should be a short imperative summary in English.

## Directory creation

```bash
python3 ".claude/scripts/spec_artifact.py" \
  --repo "." --create --title "gwt-spec: <description>"
```

This creates:

```text
specs/SPEC-{id}/
  metadata.json
```

### metadata.json structure

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

## Seeding spec.md

After directory creation, write spec.md populated from the intake memo and
domain model summary.

### Template

```markdown
# Feature Specification: <title>

## Background

<from intake memo: Request + Why now>

## Ubiquitous Language

<from domain model: term definitions>

| Term | Definition |
|---|---|
| <term> | <definition> |

## User Stories

### User Story 1 - <title> (Priority: P1)

As a <actor>, I want to <goal>, so that <benefit>.

## Acceptance Scenarios

1. Given <precondition>, when <action>, then <expected result>.

## Edge Cases

- <edge case description>

## Functional Requirements

- FR-001: <requirement>

## Non-Functional Requirements

- NFR-001: <requirement>

## Success Criteria

- SC-001: <measurable criterion>
```

### Rules for seeding

- Populate from the intake memo and domain model. Do not invent requirements.
- Use `[NEEDS CLARIFICATION: <question>]` for unknowns instead of guessing.
- Include the Ubiquitous Language section from Phase 2.
- Map user stories to the entities and BCs identified in Phase 2.
- Do not create plan.md or tasks.md at this phase.

### Upload

```bash
cat <<'SPEC_EOF' > /tmp/spec.md
<spec content>
SPEC_EOF

python3 ".claude/scripts/spec_artifact.py" \
  --repo "." --spec "<id>" --upsert \
  --artifact "doc:spec.md" --body-file /tmp/spec.md
```

## Post-registration

After creating the SPEC and seeding spec.md, proceed directly to Phase 4
(Clarification). Do not stop unless the user explicitly requested register-only.

## Full SPEC directory structure (for reference)

Later phases will add these files:

```text
specs/SPEC-{id}/
  metadata.json      # created in Phase 3
  spec.md            # created in Phase 3
  plan.md            # created by gwt-plan
  tasks.md           # created by gwt-plan/gwt-tasks
  research.md        # created by gwt-plan
  data-model.md      # created by gwt-plan
  quickstart.md      # created by gwt-plan
  contracts/         # created by gwt-plan
  checklists/        # created by gwt-plan
```
