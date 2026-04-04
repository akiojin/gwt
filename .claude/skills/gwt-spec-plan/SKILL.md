---
name: gwt-spec-plan
description: "This skill should be used when the user wants to generate planning artifacts for a SPEC, says 'plan this spec', 'generate a plan', 'create planning artifacts', '計画を作って', 'SPECのプランを生成して', or when a clarified spec.md needs implementation planning. It generates plan.md, research.md, data-model.md, quickstart.md, and contracts/* including a constitution check."
allowed-tools: Bash, Read, Glob, Grep, Edit, Write
argument-hint: "[spec-id]"
---

# gwt SPEC Plan

Translate a clarified `spec.md` into implementation-ready planning artifacts including plan.md, research.md, data-model.md, quickstart.md, and contracts.

- If `spec.md` still has critical clarification gaps, use `gwt-spec-clarify` first.
- If the target SPEC does not exist, use `gwt-spec-register` first.
- If planning artifacts already exist but are stale, update them instead of recreating them blindly.
- Prefer repairing obviously incomplete planning artifacts over stopping the workflow.

## Required inputs

- `spec.md` artifact from the target SPEC directory
- Repo-level constitution: `memory/constitution.md`

## Required outputs

Create or update these artifacts in the SPEC directory:

- `plan.md`
- `research.md`
- `data-model.md`
- `quickstart.md`
- `contracts/<name>` when interface or schema details are needed

## `plan.md` structure

`plan.md` must contain:

- Summary
- Technical Context
- Constitution Check
- Project Structure
- Complexity Tracking
- Phased Implementation

## Workflow

1. **Read the source artifacts.**
   - Load `spec.md` and `.gwt/memory/constitution.md`.
   - Refuse to continue only when `spec.md` is missing or a user decision still blocks planning.

2. **Establish technical context.**
   - Identify affected files, modules, services, and external constraints.
   - Record assumptions explicitly.

3. **Run the constitution check.**
   - Evaluate the work against `.gwt/memory/constitution.md`.
   - If a rule is violated, either redesign or record the reason in `Complexity Tracking`.

4. **Produce supporting artifacts.**
   - `research.md`: unknowns, tradeoff decisions, external findings
   - `data-model.md`: entities, shapes, lifecycle, invariants
   - `quickstart.md`: minimum validation flow for reviewers and implementers
   - `contracts/*`: only when external or internal interfaces need a stable contract

5. **Write `plan.md`.**
   - Describe phases in implementation order.
   - Keep it technical and decision-complete.

6. **Continue into task generation.**
   - Return the updated planning artifacts to `gwt-spec-ops`, or proceed directly to `gwt-spec-tasks` when the workflow is already in motion.
   - Tasks are generated from the clarified spec and plan artifacts, not from guesswork.

## Exit criteria

Planning is complete only when:

- `Constitution Check` is present and non-empty
- Phases reflect a coherent build order
- Supporting artifacts cover real decision points
- The next implementer can generate tasks without inventing architecture

## Operations

```bash
python3 ".claude/skills/gwt-spec-ops/scripts/spec_artifact.py" \
  --repo "." \
  --spec "<id>" \
  --upsert \
  --artifact "doc:plan.md" \
  --body-file /tmp/plan.md

python3 ".claude/skills/gwt-spec-ops/scripts/spec_artifact.py" \
  --repo "." \
  --spec "<id>" \
  --upsert \
  --artifact "doc:research.md" \
  --body-file /tmp/research.md

python3 ".claude/skills/gwt-spec-ops/scripts/spec_artifact.py" \
  --repo "." \
  --spec "<id>" \
  --upsert \
  --artifact "doc:data-model.md" \
  --body-file /tmp/data-model.md

python3 ".claude/skills/gwt-spec-ops/scripts/spec_artifact.py" \
  --repo "." \
  --spec "<id>" \
  --upsert \
  --artifact "doc:quickstart.md" \
  --body-file /tmp/quickstart.md
```
