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

- [ ] TEST: Unit test for `action_index_specs` — indexes 3 SPECs, verifies collection count
- [ ] TEST: Unit test for text extraction from SPEC artifacts (`spec.md`, `plan.md`, `tasks.md`, `analysis.md`)
- [ ] IMPL: Add `action_index_specs()` function in gwt-core
  - File: `crates/gwt-core/src/spec/search.rs`
- [ ] IMPL: Text extraction utility for SPEC artifact files, including `analysis.md`

### 1.2 ChromaDB Search [P]

- [ ] TEST: Unit test for `action_search_specs` — query returns ranked results with scores
- [ ] TEST: Unit test for empty query returns empty results
- [ ] IMPL: Add `action_search_specs(query: &str, top_n: usize)` function
  - File: `crates/gwt-core/src/spec/search.rs`
- [ ] IMPL: `SearchResult` struct with spec_id, title, score, snippet

### 1.3 Search UI

- [ ] TEST: Snapshot test for search input rendering in SPECs tab
- [ ] TEST: Snapshot test for search results list rendering
- [ ] IMPL: Add search input widget to SPECs tab header
  - File: `crates/gwt-tui/src/screens/specs.rs`
- [ ] IMPL: Search results display with id, title, relevance score
- [ ] IMPL: Keybinding (`/`) to activate search mode

## Phase 2: Agent Launch from SPEC Detail

### 2.1 Context Builder [P]

- [ ] TEST: Unit test for SPEC context assembly (id, title, spec.md content)
- [ ] TEST: Unit test for branch name suggestion from SPEC title
- [ ] IMPL: Add `SpecAgentContext` struct and builder
  - File: `crates/gwt-core/src/spec/agent_context.rs`
- [ ] IMPL: Branch name suggestion: title to kebab-case conversion

### 2.2 Launch Wizard Integration

- [x] TEST: Integration test for Shift+Enter keybinding in SPEC detail
- [x] IMPL: Add Shift+Enter handler to SPEC detail view
  - File: `crates/gwt-tui/src/screens/specs.rs`
- [x] IMPL: Connect to agent launch wizard with pre-filled SPEC context
- [x] IMPL: Auto-fill branch name in wizard

## Phase 3: SPEC Editing

### 3.1 Phase/Status Editor [P]

- [x] TEST: Unit test for metadata.json update (phase change)
- [ ] TEST: Unit test for metadata.json update (status change)
- [x] IMPL: Add `update_spec_metadata(id, field, value)` function
  - File: `crates/gwt-core/src/spec/metadata.rs`
- [ ] IMPL: Selection popup menu widget for phase/status

### 3.2 Content Editor [P]

- [ ] TEST: Unit test for section-level parsing of spec.md
- [x] TEST: Unit test for section content replacement and file write
- [x] IMPL: Section parser for markdown files (heading-delimited sections)
- [ ] IMPL: Inline text editor widget for section content
  - File: `crates/gwt-tui/src/widgets/section_editor.rs`

### 3.3 Edit Mode Integration

- [ ] TEST: Snapshot test for edit mode UI (phase/status selection + content editing)
- [x] IMPL: Add edit mode toggle to SPEC detail view
  - File: `crates/gwt-tui/src/screens/specs.rs`
- [ ] IMPL: Save confirmation dialog
- [x] IMPL: Write changes to disk on confirm

## Phase 4: Integration Testing

- [ ] TEST: End-to-end test: index SPECs, search, verify results
- [x] TEST: End-to-end test: launch agent from SPEC detail with correct context
- [x] TEST: End-to-end test: edit SPEC phase, verify metadata.json updated
- [ ] TEST: Regression test: existing SPEC list and detail views unaffected
