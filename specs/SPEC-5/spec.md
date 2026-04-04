# SPEC-5: Local SPEC Management — List, Detail, Search, Edit, Agent Launch

## Background

gwt manages local SPEC artifacts (`specs/SPEC-{id}/`) including `spec.md`, `plan.md`, `tasks.md`, supporting docs, and a persisted `analysis.md`. The management shell now exposes a live Specs tab again, and the shell can load local `metadata.json` entries, open detail, and launch the wizard with SPEC id/title/spec.md context plus a title-derived branch seed. The live shell now exposes phase edit (`e`) and status edit (`s`) as constrained selection menus in SPEC detail, keeps raw active-file edit available with `E`, supports section-scoped `spec.md` editing by selecting a `##` section with `Up/Down` before pressing `Ctrl+e`, and now routes read-only artifact detail through the shared markdown renderer. Semantic search remains incomplete.

## User Stories

### US-1 (P0): Browse Local SPECs List — PARTIALLY IMPLEMENTED

As a developer, I want to browse all local SPECs in a list so that I can see the status of all specifications at a glance.

**Acceptance Scenarios:**

- AC-1.1: SPEC list shows id, title, status, and phase from metadata.json
- AC-1.2: List is sorted by SPEC id (ascending)
- AC-1.3: List loads under 500ms for 100 SPECs

### US-2 (P0): View SPEC Detail — PARTIALLY IMPLEMENTED

As a developer, I want to view SPEC detail (`spec.md`, `plan.md`, `tasks.md`, `analysis.md`, etc.) with markdown rendering so that I can read specifications inline.

**Acceptance Scenarios:**

- AC-2.1: Detail view renders markdown for all artifact types (`spec.md`, `plan.md`, `tasks.md`, `analysis.md`, `research.md`)
- AC-2.2: Tab or keybinding switches between artifact files within a SPEC
- AC-2.3: Pressing Esc returns to the SPEC list

### US-3 (P1): Search SPECs by Semantic Query — NOT IMPLEMENTED

As a developer, I want to search SPECs by semantic query so that I can find relevant specifications without knowing exact titles.

**Acceptance Scenarios:**

- AC-3.1: Search input accepts free-text query
- AC-3.2: Results are ranked by relevance score from ChromaDB
- AC-3.3: Search returns results under 2 seconds
- AC-3.4: Results display SPEC id, title, and relevance score

### US-4 (P1): Launch Agent from SPEC Detail — PARTIALLY IMPLEMENTED

As a developer, I want to launch an agent session from SPEC detail view (Shift+Enter) so that I can start implementing a SPEC immediately.

**Acceptance Scenarios:**

- AC-4.1: Shift+Enter on SPEC detail opens agent launch wizard
- AC-4.2: Agent launch pre-fills SPEC context (id, title, spec.md content)
- AC-4.3: Agent launch auto-suggests branch name from SPEC title (e.g., `feature/spec-5-local-spec-management`)
- AC-4.4: Canceling the wizard returns to SPEC detail

### US-5 (P1): Edit SPEC Artifacts from TUI — IMPLEMENTED

As a developer, I want to edit SPEC artifacts (status, phase, content) from within the TUI so that I can update specifications without switching to an editor.

**Acceptance Scenarios:**

- AC-5.1: Phase/status can be updated via a selection menu in SPEC detail
- AC-5.2: metadata.json is updated on save
- AC-5.3: Inline edit of spec.md sections is supported
- AC-5.4: Changes are written to disk on confirm

### US-6 (P0): Generate New SPEC via SpecKit Wizard — IMPLEMENTED

As a developer, I want to generate a new SPEC through a guided wizard so that I can create well-structured specifications.

**Acceptance Scenarios:**

- AC-6.1: SpecKit Wizard follows Clarify -> Specify -> Plan -> Tasks -> Done flow
- AC-6.2: Each step validates before proceeding to the next
- AC-6.3: Generated SPEC artifacts are written to `specs/SPEC-{id}/`

## Functional Requirements

| ID | Requirement | Priority | Status |
|----|-------------|----------|--------|
| FR-001 | SPEC list shows id, title, status, phase from metadata.json | P0 | Implemented |
| FR-002 | Detail view renders markdown for all artifact types, including `analysis.md` | P0 | Implemented |
| FR-003 | Semantic search via ChromaDB index (action_index_specs, action_search_specs) | P1 | Not Implemented |
| FR-004 | Search results ranked by relevance score | P1 | Not Implemented |
| FR-005 | Shift+Enter on SPEC opens agent launch wizard with SPEC context | P1 | Implemented |
| FR-006 | Agent launch auto-suggests branch name from SPEC title | P1 | Implemented |
| FR-007 | SPEC edit: update phase/status in metadata.json | P1 | Implemented |
| FR-008 | SPEC edit: inline edit of spec.md sections | P1 | Implemented |
| FR-009 | SpecKit Wizard: Clarify -> Specify -> Plan -> Tasks -> Done | P0 | Implemented |

## Non-Functional Requirements

| ID | Requirement |
|----|-------------|
| NFR-001 | SPEC list loads under 500ms for 100 SPECs |
| NFR-002 | Semantic search returns results under 2 seconds |

## Design Notes

- Semantic search uses ChromaDB for vector embeddings; indexing is triggered by `action_index_specs` and searching by `action_search_specs`
- Agent launch from SPEC detail follows the same wizard pattern as Issue-based agent launch (reference archived SPEC-1785 for details)
- SPEC editing writes directly to the filesystem; no intermediate database
- `spec.md` section editing targets the first-level body sections (`## ...`), identifies the selected section by parsed section order rather than heading text alone, ignores fenced-code pseudo-headings, and preserves nested `### ...` content within the selected section
- `Ctrl+e` edits the selected `spec.md` section body, while `E` keeps raw full-file editing available for `spec.md` and the other artifact tabs
- `analysis.md` is a persisted local artifact and must stay aligned with the current readiness judgment

## Success Criteria

1. Semantic search returns relevant SPECs for free-text queries within 2 seconds
2. Agent launch from SPEC detail pre-fills context and suggests a branch name
3. SPEC phase/status can be edited and persisted from the TUI
4. All existing functionality (US-1, US-2, US-6) continues without regression
