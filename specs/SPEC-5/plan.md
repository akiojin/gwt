# SPEC-5: Implementation Plan

## Phase 1: Semantic Search

**Goal:** Enable ranked free-text search over local SPEC files, including persisted readiness artifacts, directly inside the Specs tab.

### Approach

- Reuse the existing Specs search input and detail/list flow
- Rank matches across `metadata.json` fields plus local artifact bodies
- Show score + snippet inline in the Specs list without adding a background index

### Components

1. **Query tokenizer** — Split free-text input into case-insensitive tokens
2. **Relevance scorer** — Rank matches across id/title/phase/status and local artifact files
3. **Search UI** — Keep the existing search header, replace the list with ranked results showing score + snippet

### Key Decisions

- The initial slice stays inside `screens/specs.rs` and does not add a new dependency or background worker
- Relevance is good-enough local ranking, not embedding-based semantic similarity

## Phase 2: Agent Launch from SPEC Detail

**Goal:** Allow launching an agent session directly from SPEC detail view when the screen is reachable from the live shell.

### Approach

- Reference archived SPEC-1785 for agent launch wizard design
- Add Shift+Enter keybinding to SPEC detail view
- Pre-fill agent context with SPEC id, title, and spec.md content
- Auto-suggest branch name derived from SPEC title

### Components

1. **Keybinding handler** — Detect Shift+Enter in SPEC detail mode
2. **Context builder** — Assemble SPEC context for agent launch
3. **Branch name suggestion** — Convert SPEC title to kebab-case branch name

## Phase 3: SPEC Editing

**Goal:** Enable editing SPEC artifacts (status, phase, content) from within the TUI, including persisted readiness artifacts when appropriate.

### Approach

- Add edit mode to SPEC detail view
- Phase/status editing via selection popup menu
- Content editing via inline text editing of spec.md sections

### Components

1. **Phase/status editor** — Selection menu overlay with available phases/statuses
2. **metadata.json writer** — Update and save metadata.json on confirm
3. **Content editor** — Section-level inline editing for spec.md
4. **File writer** — Write modified content back to disk

### Key Decisions

- Inline editing is section-level (not full-file) to keep the UX simple
- Changes are only persisted on explicit confirm (not auto-save)

## Dependencies

- Agent launch infrastructure — existing wizard pattern from Issue-based launch
- Archived SPEC-1785 — reference for agent launch design
