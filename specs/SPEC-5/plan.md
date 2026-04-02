# SPEC-5: Implementation Plan

## Phase 1: Semantic Search

**Goal:** Enable semantic search over local SPEC files using ChromaDB.

### Approach

- Implement `action_index_specs` to read all SPEC artifacts and index them in ChromaDB
- Implement `action_search_specs` to query ChromaDB with free-text and return ranked results
- Add search UI to the SPECs tab (search input + results list)

### Components

1. **Indexing** — Scan `specs/SPEC-{id}/` directories, extract text from spec.md/plan.md/tasks.md, upsert into ChromaDB collection
2. **Search query** — Accept free-text, query ChromaDB, return top-N results with relevance score
3. **Search UI** — Input field at top of SPECs tab, results replace list temporarily

### Key Decisions

- ChromaDB is used for vector storage because it runs locally without external dependencies
- Index is rebuilt on demand (not auto-updated) to avoid background overhead

## Phase 2: Agent Launch from SPEC Detail

**Goal:** Allow launching an agent session directly from SPEC detail view.

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

**Goal:** Enable editing SPEC artifacts (status, phase, content) from within the TUI.

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

- ChromaDB — required for semantic search (Phase 1)
- Agent launch infrastructure — existing wizard pattern from Issue-based launch
- Archived SPEC-1785 — reference for agent launch design
