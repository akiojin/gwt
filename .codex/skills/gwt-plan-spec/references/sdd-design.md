# SDD (Software Design Document) Methodology

Reference for Phase 2 of `gwt-plan-spec`. Describes how to produce architecture design
artifacts using SDD methodology.

## Purpose

Bridge the gap between specification (what to build) and task decomposition (how to
build it) by producing a concrete technical design that:

- Makes component boundaries explicit
- Defines stable interface contracts
- Documents data models and their invariants
- Describes key interaction flows

## Component Design

### What to document

For each new or significantly modified component:

- **Name** — crate, module, or struct name
- **Responsibility** — single sentence describing what it owns
- **Ownership boundary** — what data and behavior it exclusively controls
- **Dependencies** — what it requires from other components
- **Public surface** — traits, functions, or messages it exposes

### Guidelines

- Prefer modifying existing components over introducing new ones
- A component that cannot be described in one sentence is too large — split it
- Record the reason in `Complexity Tracking` when adding a new abstraction layer
- Follow the project's existing module hierarchy (`gwt-core`, `gwt-tui`)

## Interface Contracts

### When to write contracts

Write a contract file in `contracts/` when:

- An interface crosses crate boundaries (e.g., `gwt-core` to `gwt-tui`)
- A public API will be consumed by multiple callers
- A message format or protocol needs stability guarantees
- An external system boundary is involved

### Contract content

Each contract should specify:

- **Participants** — which components are on each side
- **Method signatures or message shapes** — types, parameters, return values
- **Preconditions** — what must be true before calling
- **Postconditions** — what the caller can expect after success
- **Error cases** — how failures are communicated
- **Versioning notes** — if the contract may evolve

### Guidelines

- Keep contracts minimal — only what callers need to know
- Use Rust type signatures when possible for precision
- Do not duplicate implementation details — contracts describe the boundary, not the internals

## Data Model

### What to document in `data-model.md`

- **Entities** — name, fields, types
- **Shapes** — struct layout, enum variants, serialization format
- **Lifecycle** — creation, state transitions, deletion/archival
- **Invariants** — rules that must always hold (e.g., "name is non-empty",
  "status transitions are unidirectional")
- **Storage** — where the data lives (file, memory, Git metadata)

### Guidelines

- Document only entities that the SPEC introduces or modifies
- Reference existing data models by path rather than duplicating definitions
- Mark nullable/optional fields explicitly
- Note migration requirements if existing data shapes change

## Sequence Descriptions

### Format

Describe interactions in plain text. No diagram syntax required. Structure each flow as:

```text
Flow: <name>
Trigger: <what initiates this flow>
Steps:
1. <Component A> does <action> with <data>
2. <Component B> receives <data> and does <action>
3. <Component A> receives <result> and does <action>
Error path:
- If step 2 fails: <what happens>
```

### What to describe

- Happy path for each user story
- Error/fallback paths for non-trivial failure modes
- Async or concurrent interactions where ordering matters

### Guidelines

- One flow per user story or acceptance scenario
- Keep steps concrete — name the function, struct, or message
- Note where parallelism is possible
- Omit trivial flows (e.g., simple CRUD with no business logic)
