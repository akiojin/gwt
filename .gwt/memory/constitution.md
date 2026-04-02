# gwt Constitution

## Core Rules

### 1. Spec Before Implementation

- `feat`, `fix`, and `refactor` work must not enter implementation until the relevant
  `gwt-spec` container has a usable `spec.md`, `plan.md`, `tasks.md`, and an analysis pass.
- If critical ambiguity remains, record it as `[NEEDS CLARIFICATION: ...]` and stop before code.

### 2. Test-First Delivery

- Every user story must map to verification work before or alongside implementation.
- Prefer contract, integration, and end-to-end checks that prove the acceptance scenarios.

### 3. No Workaround-First Changes

- Do not accept speculative fixes or hand-wavy plans.
- Root cause, tradeoffs, and impacted surfaces must be explicit in the spec or plan artifacts.

### 4. Minimal Complexity

- Choose the simplest approach that satisfies the accepted requirements.
- If the design introduces extra components, abstractions, or migrations, record the reason in
  `Complexity Tracking`.

### 5. Verifiable Completion

- A task is not complete until the relevant checks have run successfully or an explicit exception
  is documented with reason, fallback verification, and residual risk.

### 6. SPEC vs Issue Separation

- **SPEC = Feature specification.** Defines new functionality, design, or architecture.
  One SPEC per cohesive feature (e.g., "Voice Input", "Docker Support", not "fix voice crash").
- **Issue = Work item.** Bug fixes, tasks, and improvements are GitHub Issues linked to a SPEC.
  Never create a SPEC for a bug fix — file an Issue against the relevant SPEC instead.
- **Granularity rule:** A SPEC should be decomposable into 3-15 tasks. If fewer, merge into
  a parent SPEC. If more, split into child SPECs with clear scope boundaries.
- **No duplicate scope:** Before creating a SPEC, search existing SPECs (`gwt-spec-search`)
  and Issues (`gwt-issue-search`). Reuse an existing owner when one clearly fits.
- **SPEC categories:** CORE-TUI, AGENT, GIT, DOCKER, GITHUB, CONFIG, ASSISTANT, SEARCH,
  BUILD, NOTIFICATION, VOICE, DESIGN. Each SPEC belongs to exactly one category.

## Required Plan Gates

Every `plan.md` must answer these questions:

1. What files/modules are affected?
2. What constraints from this constitution apply?
3. Which risks or complexity additions are accepted, and why?
4. How will the acceptance scenarios be verified?
