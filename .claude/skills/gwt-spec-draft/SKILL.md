---
name: gwt-spec-draft
description: "Launch a SPEC drafting session on the develop branch. Brainstorm requirements with the user, create one or more SPECs with feature branches, and return to develop. Use when user says 'new SPEC', 'brainstorm a feature', 'draft a spec', or wants to start SPEC creation from scratch with a brainstorming phase."
---

# gwt SPEC Drafting Skill

Use this skill to brainstorm and draft new SPECs on the develop branch.
This is the recommended entry point when the user wants to create new work
items starting from a high-level idea rather than a well-defined requirement.

## Workflow

### Phase 1: Brainstorming

1. **Clarify the idea** with the user through structured interview:
   - What problem does this solve?
   - Who is the target user?
   - What does success look like?
   - Are there existing SPECs or Issues that overlap?

2. **Search for duplicates** (mandatory):
   - Use `gwt-issue-search` to check existing Issues
   - Use `gwt-spec-search` to check existing SPECs
   - If an existing SPEC covers this scope, recommend using `gwt-spec-ops` instead

3. **Scope definition**:
   - Determine if this is one SPEC or multiple SPECs
   - For large features, split into focused SPECs with clear boundaries
   - Each SPEC should be independently implementable

### Phase 2: SPEC Creation

For each SPEC identified in Phase 1:

1. **Create the SPEC** using `gwt-spec-register`:
   - Follow the standard SPEC registration workflow
   - Ensure spec.md has user stories, acceptance scenarios, and functional requirements

2. **Create a feature branch** for each SPEC:
   - Branch name: `feature/feature-{N}` where N is the SPEC ID
   - Create from develop: `git checkout -b feature/feature-{N} develop`
   - Commit SPEC files to the feature branch
   - Return to develop: `git checkout develop`

3. **Continue to planning** if the user wants:
   - Use `gwt-spec-plan` to generate plan.md
   - Use `gwt-spec-tasks` to generate tasks.md
   - Use `gwt-spec-analyze` to verify completeness

### Phase 3: Handoff

After all SPECs are created:

1. **Summary**: Present a summary of all created SPECs with their feature branches
2. **Priority**: Help the user decide which SPEC to implement first
3. **Implementation**: If requested, proceed to `gwt-spec-implement` on the
   highest-priority SPEC's feature branch

## Rules

- **Never commit to develop directly.** The pre-commit hook blocks this.
  All SPEC artifacts must be committed on their respective feature branches.
- **One SPEC per feature branch.** Do not mix multiple SPECs on a single branch.
- **Return to develop** between SPEC creations to keep branches independent.
- **Interview depth**: Ask substantive technical questions, not surface-level ones.
  Probe into UX tradeoffs, technical constraints, and edge cases.

## Example Session

```text
User: "I want to add keyboard shortcuts to the app"

Agent: [Uses gwt-issue-search and gwt-spec-search to check for duplicates]
Agent: "I found no existing SPECs for keyboard shortcuts. Let me ask some
        clarifying questions..."
Agent: [Conducts structured interview]
Agent: "Based on our discussion, I recommend splitting this into two SPECs:
        1. SPEC-1800: Global keyboard shortcut framework
        2. SPEC-1801: Screen-specific shortcut bindings"
Agent: [Creates SPEC-1800 on feature/feature-1800]
Agent: [Creates SPEC-1801 on feature/feature-1801]
Agent: "Both SPECs are ready. I recommend starting with SPEC-1800 as it's
        the foundation. Shall I proceed with planning?"
```
