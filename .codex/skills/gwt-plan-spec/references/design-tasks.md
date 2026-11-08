## Phase 2: Architecture Design (SDD)

Design the solution using Software Design Document methodology before decomposing tasks.

> See [references/sdd-design.md](sdd-design.md) for full methodology.

1. **Component design.** Identify new and modified components, their responsibilities,
   and ownership boundaries. Keep the design minimal — no unnecessary abstractions.

2. **Interface contracts.** Define public APIs, message formats, and protocol boundaries
   between components. Write stable contracts to `contracts/*` when interfaces cross
   module or crate boundaries.

   A new or changed blocking gate is not implementation-ready unless the plan
   and tasks name all four required acceptance elements:
   - an **agent-reachable recovery operation**
   - **diagnostic visibility** for both the requirement and current state
   - a **positive test** proving the invalid state is rejected
   - a **false-positive negative test** proving legitimate work is not rejected
   Missing any element is a quality-gate failure; a warning-only policy is not
   a blocking gate.

3. **Data model.** Document entities, their shapes, lifecycle states, and invariants.
   Write to `data-model.md`.

4. **Sequence descriptions.** Describe key interaction flows in plain text — which
   component calls what, in what order, with what data. No diagram syntax required;
   clarity and completeness matter.

5. **Produce supporting artifacts.**
   - `research.md`: unknowns, tradeoff decisions, external findings
   - `quickstart.md`: minimum validation flow for reviewers and implementers

6. **Write `plan.md`.** Structure:
   - Summary
   - Technical Context (from Phase 1)
   - Constitution Check
   - Architecture Design (component, interface, data model, sequences)
   - Project Structure
   - Complexity Tracking
   - Phased Implementation

## Phase 3: Task Decomposition

Turn the architecture and plan into executable work items.

> See [references/task-decomposition.md](task-decomposition.md) for full rules.

1. **Lay out phase order.** Canonical ordering:
   - **Setup** — tooling, configuration, scaffolding
   - **Foundational** — shared infrastructure, core types
   - **User Story phases** (`US1`, `US2`, ...) — story-specific implementation
   - **Polish / Cross-Cutting** — cleanup, documentation, integration tests

2. **Generate test-first tasks.** For each user story:
   - Add test/verification tasks before or alongside implementation tasks
   - Include contract, integration, and e2e coverage when the spec implies it

3. **Add implementation tasks.** Each task must include:
   - **Task ID** in `T-NNN` format
   - **`[P]` marker** when parallelizable (only when write scopes do not overlap)
   - **Linked user story ID** where applicable
   - **Exact file path or module**
   - **Concrete action** — no vague descriptions

4. **Validate traceability.**
   - Every user story has implementation and verification tasks
   - Every acceptance scenario has verification coverage
   - Every declared contract/data-model change has matching tasks

5. **Write `tasks.md`.**
