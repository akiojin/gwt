# DDD Domain Discovery Reference

Detailed logic for the domain discovery phase of gwt-discussion.

## Purpose

Apply Domain-Driven Design modeling between intake and registration to ensure
the SPEC is well-scoped, uses consistent terminology, and maps cleanly to the
codebase architecture.

## Step 2.1: Bounded Context identification

### What is a Bounded Context

A Bounded Context (BC) is a boundary within which a domain model is consistent
and a particular Ubiquitous Language applies. In the gwt project, BCs roughly
map to crate boundaries and major subsystems.

### Known BCs in gwt

| BC | Crate / Location | Responsibility |
|---|---|---|
| Core | `gwt-core` | Git operations, PTY management, configuration, data model |
| TUI | `gwt-tui` | Terminal UI rendering, input handling, screen management |
| Specs | `specs/` | Feature specification artifacts |
| Skills | `.claude/skills/` | Agent skill definitions |

### Identification process

1. Review the intake memo for functional areas touched.
2. Map each area to an existing BC.
3. If a function does not fit any existing BC, propose a new BC with:
   - Name (short, noun-based)
   - Single-sentence responsibility
   - Boundary definition (what it owns, what it does not)
4. Check for BC overlap: if the feature straddles two BCs, document the
   integration point rather than merging concerns.

### Red flags

- A feature that modifies entities in 3+ BCs likely needs splitting.
- A "utility" BC is usually a sign of missing domain modeling.
- If two BCs share the same entity name with different semantics, the language
  needs alignment.

## Step 2.2: Entity and aggregate mapping

### Definitions

- **Entity**: an object with identity that persists over time (e.g., Worktree, Branch, Pane).
- **Value Object**: an immutable object defined by its attributes (e.g., CommitHash, FilePath).
- **Aggregate Root**: the entry point entity that enforces invariants for a cluster.

### Process

1. List entities the feature introduces or modifies.
2. For each entity, identify:
   - Which BC it belongs to
   - Whether it is an entity or value object
   - Its aggregate root (if part of a cluster)
3. Map relationships:
   - **owns**: parent-child within the same aggregate
   - **references**: cross-aggregate reference by ID
   - **depends-on**: runtime dependency (e.g., needs data from another BC)

### Output format

```markdown
### Entities
| Entity | Type | BC | Aggregate Root |
|---|---|---|---|
| Worktree | Entity | Core | Worktree |
| Branch | Value Object | Core | Worktree |
```

### Relationships
```markdown
| From | To | Type | Description |
|---|---|---|---|
| Worktree | Branch | owns | A worktree has one active branch |
| Pane | Worktree | references | A pane displays a worktree |
```

## Step 2.3: Ubiquitous Language

### Purpose

Establish a shared vocabulary that is used consistently in spec.md, code, tests,
and conversation. Prevents confusion from synonyms or overloaded terms.

### Process

1. Extract domain terms from the intake memo and interview answers.
2. Check each term against existing codebase usage:
   - Search with `gwt-project-search` for existing definitions.
   - Check struct/enum/function names in `gwt-core` and `gwt-tui`.
3. For each term, write a one-sentence definition.
4. Flag conflicts:
   - Term used differently in different parts of the codebase.
   - Term that conflicts with a common programming term (e.g., "task" as domain
     vs "task" as async runtime).
5. Resolve conflicts by choosing the canonical term and noting aliases.

### Output format

```markdown
## Ubiquitous Language
| Term | Definition | Aliases | Conflicts |
|---|---|---|---|
| Worktree | A git worktree with its own working directory | workspace | -- |
| Pane | A TUI panel displaying a worktree or agent | -- | tmux pane (different concept) |
```

This glossary becomes the `## Ubiquitous Language` section in spec.md.

## Step 2.4: BC boundary check (granularity gate)

### SPEC granularity validation

Use the domain model to validate that the proposed SPEC has the right scope:

**Single-BC rule**: a SPEC should map to one primary Bounded Context. If it
touches multiple BCs, consider:

- Is the cross-BC work a thin integration layer? (OK to keep as one SPEC)
- Does each BC require significant new entities or logic? (Split into per-BC SPECs)

**SPEC vs Issue decision**:

| Signal | SPEC | Issue |
|---|---|---|
| New user-facing functionality | Yes | -- |
| Architecture or design decisions | Yes | -- |
| Bug fix | -- | Yes (link to parent SPEC) |
| One-off chore | -- | Yes |

SPEC scope is determined by feature cohesion, not task count.
Implementation phasing is handled by `gwt-plan-spec`.

### When to split a SPEC

Split only when the SPEC spans **multiple distinct features** (not when task count is high):

1. The SPEC touches multiple BCs with significant independent logic in each.
2. Each part could be specified and delivered independently.
3. Define integration contracts between child SPECs.

### When to merge into a parent SPEC

1. The work is a natural extension of an existing feature SPEC.
2. Merge the new user stories into the parent's spec.md.

### Domain model summary output

Record for use in Phase 3:

```markdown
## Domain Model Summary

### Bounded Contexts
- <BC name>: <responsibility>

### Entities
- <Entity>: <description> (BC: <name>)

### Ubiquitous Language
- <Term>: <definition>

### Integration Points
- <BC-A> -> <BC-B>: <interaction description>

### Granularity Assessment
- Primary BC: <name>
- Task estimate: <N>
- User story count: <N>
- Verdict: SPEC | Issue | Split into <N> SPECs
```
