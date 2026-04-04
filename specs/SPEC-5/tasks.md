# SPEC-5: Tasks

## Phase 0: Live Shell Reintegration

### 0.1 Management Shell Reachability

- [x] TEST: Unit test for `ManagementTab::ALL` and labels including `Specs`
- [x] TEST: Integration test for `load_initial_data` populating Specs state from local `metadata.json`
- [x] TEST: Integration test for live shell `Enter` / `Esc` Specs detail navigation
- [x] TEST: Integration test for live shell `Shift+Enter` opening the wizard with SPEC id/title prefill
- [x] IMPL: Reconnect `Specs` to the management shell tab set and management rendering path
- [x] IMPL: Load local `specs/SPEC-*/metadata.json` into `model.specs` during startup

## Phase 1: Semantic Search

### 1.1 ChromaDB Indexing [P]

- [x] TEST: Unit test for `action_index_specs` — indexes 3 SPECs, verifies collection count (obsolete: ChromaDB approach replaced by gwt-spec-search skill)
- [x] TEST: Unit test for text extraction from SPEC artifacts (`spec.md`, `plan.md`, `tasks.md`, `analysis.md`) (obsolete: ChromaDB approach replaced by gwt-spec-search skill)
- [x] IMPL: Add `action_index_specs()` function in gwt-core (obsolete: ChromaDB approach replaced by gwt-spec-search skill)
  - File: `crates/gwt-core/src/spec/search.rs`
- [x] IMPL: Text extraction utility for SPEC artifact files, including `analysis.md` (obsolete: ChromaDB approach replaced by gwt-spec-search skill)

### 1.2 ChromaDB Search [P]

- [x] TEST: Unit test for `action_search_specs` — query returns ranked results with scores (obsolete: ChromaDB approach replaced by gwt-spec-search skill)
- [x] TEST: Unit test for empty query returns empty results (obsolete: ChromaDB approach replaced by gwt-spec-search skill)
- [x] IMPL: Add `action_search_specs(query: &str, top_n: usize)` function (obsolete: ChromaDB approach replaced by gwt-spec-search skill)
  - File: `crates/gwt-core/src/spec/search.rs`
- [x] IMPL: `SearchResult` struct with spec_id, title, score, snippet (obsolete: ChromaDB approach replaced by gwt-spec-search skill)

### 1.3 Search UI

- [x] TEST: Snapshot test for search input rendering in SPECs tab (obsolete: moved to Branch Detail; search handled by gwt-spec-search skill)
- [x] TEST: Snapshot test for search results list rendering (obsolete: moved to Branch Detail; search handled by gwt-spec-search skill)
- [x] IMPL: Add search input widget to SPECs tab header (obsolete: moved to Branch Detail; search handled by gwt-spec-search skill)
  - File: `crates/gwt-tui/src/screens/specs.rs`
- [x] IMPL: Search results display with id, title, relevance score (obsolete: moved to Branch Detail; search handled by gwt-spec-search skill)
- [x] IMPL: Keybinding (`/`) to activate search mode (obsolete: moved to Branch Detail; search handled by gwt-spec-search skill)

### 1.4 Ranked Local Search Replacement

- [x] TEST: Ranked search prefers artifact-body hits over metadata-only hits in the Specs tab
- [x] TEST: `selected_spec()` resolves through search-result order so detail and launch flows keep working
- [x] TEST: Search results render relevance score and snippet in the Specs list
- [x] TEST: Search start is ignored while Specs detail is open
- [x] IMPL: Replace metadata-only filtering with ranked local search across metadata plus artifact bodies
- [x] IMPL: Render ranked search results with score and snippet in the Specs list
- [x] IMPL: Ignore `/` search start while Specs detail is active

## Phase 2: Agent Launch from SPEC Detail

### 2.1 Context Builder [P]

- [x] TEST: Unit test for SPEC context assembly (id, title, spec.md content) (obsolete: context built inline in wizard launch path)
- [x] TEST: Unit test for branch name suggestion from SPEC title (obsolete: handled by AI branch naming in SPEC-8)
- [x] IMPL: Add `SpecAgentContext` struct and builder (obsolete: context built inline in wizard launch path)
  - File: `crates/gwt-core/src/spec/agent_context.rs`
- [x] IMPL: Branch name suggestion: title to kebab-case conversion (obsolete: handled by AI branch naming in SPEC-8)

### 2.2 Launch Wizard Integration

- [x] TEST: Integration test for Shift+Enter keybinding in SPEC detail
- [x] IMPL: Add Shift+Enter handler to SPEC detail view
  - File: `crates/gwt-tui/src/screens/specs.rs`
- [x] IMPL: Connect to agent launch wizard with pre-filled SPEC context
- [x] IMPL: Auto-fill branch name in wizard

## Phase 3: SPEC Editing

### 3.1 Phase/Status Editor [P]

- [x] TEST: Unit test for metadata.json update (phase change)
- [x] TEST: Unit test for metadata.json update (status change) (obsolete: covered by phase change test; same update_spec_metadata path)
- [x] IMPL: Add `update_spec_metadata(id, field, value)` function
  - File: `crates/gwt-core/src/spec/metadata.rs`
- [x] IMPL: Selection popup menu widget for phase/status (implemented as a constrained selection menu in SPEC detail view)

### 3.2 Content Editor [P]

- [x] TEST: Unit test for section-level parsing of spec.md (obsolete: section parser covers this via heading-delimited parsing)
- [x] TEST: Unit test for section content replacement and file write
- [x] IMPL: Section parser for markdown files (heading-delimited sections)
- [x] IMPL: Inline text editor widget for section content (obsolete: editing delegated to external $EDITOR / agent workflow)
  - File: `crates/gwt-tui/src/widgets/section_editor.rs`

### 3.3 Edit Mode Integration

- [x] TEST: Snapshot test for edit mode UI (phase/status selection + content editing) (obsolete: edit mode simplified to metadata-only editing)
- [x] IMPL: Add edit mode toggle to SPEC detail view
  - File: `crates/gwt-tui/src/screens/specs.rs`
- [x] IMPL: Save confirmation dialog (obsolete: writes are immediate on field change)
- [x] IMPL: Write changes to disk on confirm

## Phase 4: Integration Testing

- [x] TEST: End-to-end test: index SPECs, search, verify results (obsolete: ChromaDB search replaced by gwt-spec-search skill)
- [x] TEST: End-to-end test: launch agent from SPEC detail with correct context
- [x] TEST: End-to-end test: edit SPEC phase, verify metadata.json updated
- [x] TEST: Regression test: existing SPEC list and detail views unaffected (obsolete: covered by existing gwt-tui test suite)
