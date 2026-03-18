---
name: gwt-spec-plan
description: Generate planning artifacts for an existing `gwt-spec`: `plan.md`, `research.md`, `data-model.md`, `quickstart.md`, and `contracts/*`, including a constitution check against `memory/constitution.md`. Use after `gwt-spec-clarify`.
---

# gwt SPEC Plan

Use this skill to translate a clarified `spec.md` into implementation-ready planning artifacts.

- If `spec.md` still has critical clarification gaps, use `gwt-spec-clarify` first.
- If the target SPEC does not exist, use `gwt-spec-register` first.
- If planning artifacts already exist but are stale, update them instead of recreating them blindly.

## Required inputs

- `spec.md` artifact from the target `gwt-spec`
- Repo-level constitution: `memory/constitution.md`

## Required outputs

Create or update these artifacts as issue comments:

- `doc:plan.md`
- `doc:research.md`
- `doc:data-model.md`
- `doc:quickstart.md`
- `contract:<name>` when interface or schema details are needed

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
   - Load `spec.md` and `memory/constitution.md`.
   - Refuse to continue if `spec.md` is missing or not clarification-ready.

2. **Establish technical context.**
   - Identify affected files, modules, services, and external constraints.
   - Record assumptions explicitly.

3. **Run the constitution check.**
   - Evaluate the work against `memory/constitution.md`.
   - If a rule is violated, either redesign or record the reason in `Complexity Tracking`.

4. **Produce supporting artifacts.**
   - `research.md`: unknowns, tradeoff decisions, external findings
   - `data-model.md`: entities, shapes, lifecycle, invariants
   - `quickstart.md`: minimum validation flow for reviewers and implementers
   - `contracts/*`: only when external or internal interfaces need a stable contract

5. **Write `plan.md`.**
   - Describe phases in implementation order.
   - Keep it technical and decision-complete.

6. **Hand off to `gwt-spec-tasks`.**
   - Tasks are generated from the clarified spec and plan artifacts, not from guesswork.

## Exit criteria

Planning is complete only when:

- `Constitution Check` is present and non-empty
- Phases reflect a coherent build order
- Supporting artifacts cover real decision points
- The next implementer can generate tasks without inventing architecture
