# SPEC セマンティック検索と検索命名規約

## Background

gwt uses ChromaDB-based semantic search via `chroma_index_runner.py` for three categories: project files, GitHub Issues, and local SPECs. The runner currently supports `index`/`search` (files) and `index-issues`/`search-issues` (Issues), but `index-specs`/`search-specs` (SPECs) is missing. Additionally, the naming convention is inconsistent across categories.

## User Stories

### US1 - Search SPECs semantically (P1)

As a developer, I want to search local SPECs by meaning (not just keyword) so that I can find related specifications before creating new ones or starting implementation.

### US2 - Consistent search API (P1)

As a developer using gwt-spec-search/gwt-issue-search/gwt-project-search skills, I want consistent action names and output formats so that I can predict the API without checking documentation.

### US3 - SPECs tab search (P2)

As a user of gwt-tui, I want to search SPECs from the SPECs management tab using the same semantic search infrastructure.

## Acceptance Scenarios

1. `--action index-specs` scans `specs/SPEC-*/` and indexes metadata + spec.md into ChromaDB `specs` collection
2. `--action search-specs --query "Docker"` returns SPEC-1552 and SPEC-1642 as top results
3. `--action search-files` works as alias for the previous `--action search`
4. `--action index-files` works as alias for the previous `--action index`
5. Output keys are unified: `fileResults`, `issueResults`, `specResults`
6. Backward compatibility: `--action index` and `--action search` still work

## Functional Requirements

- FR-001: `action_index_specs(project_root, db_path)` — scan `specs/SPEC-*/metadata.json` + `spec.md`, upsert into `specs` collection
- FR-002: `action_search_specs(db_path, query, n_results)` — query `specs` collection, return `specResults`
- FR-003: Rename `index` → `index-files`, `search` → `search-files` (keep old names as aliases)
- FR-004: Rename output key `results` → `fileResults` for file search
- FR-005: Unified output format: `{ok, {category}Results: [{id/path, title/description, distance, ...}]}`
- FR-006: argparse choices include all 8 actions: `probe`, `index-files`, `search-files`, `status`, `index-issues`, `search-issues`, `index-specs`, `search-specs` (plus aliases `index`, `search`)

## Success Criteria

- SC-001: `index-specs` indexes all 37 SPECs successfully
- SC-002: `search-specs` returns relevant results for semantic queries
- SC-003: All three search skills (gwt-project-search, gwt-issue-search, gwt-spec-search) work with consistent API
- SC-004: Backward compatibility maintained for `index`/`search` aliases
