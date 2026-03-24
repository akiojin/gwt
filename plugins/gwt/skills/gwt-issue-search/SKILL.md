---
name: gwt-issue-search
description: Semantic search over GitHub Issues and local SPECs using vector embeddings. Use when searching for existing specs, finding related issues, checking for duplicate specs, or determining which spec owns a scope. Mandatory preflight before gwt-spec-register, gwt-spec-ops, gwt-issue-register, and gwt-issue-resolve.
---

# Issue Search

gwt maintains a vector search index of GitHub Issues using ChromaDB embeddings.

## Issues search first for spec integration

When the user asks any of the following, use Issues search and local SPEC search **before** manual `gh issue list`,
title grep, or file search:

- "既存仕様を探して"
- "どの SPEC に統合するべきか"
- "関連 Issue / spec を探して"
- "Project Index の統合仕様を確認して"
- "bug / feature の過去設計を見たい"

For spec integration work, the first question is not "which file should I edit?" but
"which existing SPEC is the canonical destination?".

Minimum workflow:

1. Update the Issues index with `index-issues`
2. Search local SPECs via `index-specs` + `search-specs`
3. Run `search-issues` with 2-3 semantic queries derived from the request
4. Pick the canonical existing spec if found
5. Only fall back to creating a new spec when no suitable canonical spec exists

Suggested query patterns:

- subsystem + purpose
  - `project index issue search spec`
- user-facing problem + architecture term
  - `chroma persisted db recovery project index`
- workflow / discoverability requirement
  - `LLM should use gwt-issue-search before spec creation`

## GitHub Issues search command

First, update the Issues index (fetches Issues via `gh` CLI):

```bash
~/.gwt/runtime/chroma-venv/bin/python3 ~/.gwt/runtime/chroma_index_runner.py \
  --action index-issues \
  --db-path "$GWT_PROJECT_ROOT/.gwt/index"
```

Then search Issues semantically:

```bash
~/.gwt/runtime/chroma-venv/bin/python3 ~/.gwt/runtime/chroma_index_runner.py \
  --action search-issues \
  --db-path "$GWT_PROJECT_ROOT/.gwt/index" \
  --query "your search query" \
  --n-results 10
```

## Local SPEC search command

Index local SPEC files from `specs/SPEC-*/` directories (reads `metadata.json` + first 500 chars of `spec.md`):

```bash
~/.gwt/runtime/chroma-venv/bin/python3 ~/.gwt/runtime/chroma_index_runner.py \
  --action index-specs \
  --project-root "$GWT_PROJECT_ROOT" \
  --db-path "$GWT_PROJECT_ROOT/.gwt/index"
```

Then search local SPECs semantically:

```bash
~/.gwt/runtime/chroma-venv/bin/python3 ~/.gwt/runtime/chroma_index_runner.py \
  --action search-specs \
  --db-path "$GWT_PROJECT_ROOT/.gwt/index" \
  --query "your search query" \
  --n-results 10
```

List all local SPECs directly (no embedding needed):

```bash
python3 "${CLAUDE_PLUGIN_ROOT}/skills/gwt-spec-ops/scripts/spec_artifact.py" \
  --repo "." \
  --list-all
```

### Local SPEC directory format

```text
specs/SPEC-{N}/
  metadata.json  # {"id","title","status","phase","created_at","updated_at"}
  spec.md        # Main specification content
```

### Local SPEC search output format

```json
{"ok": true, "specResults": [
  {"spec_id": "a1b2c3d4", "title": "Add vector search", "status": "active", "phase": "implement", "dir_name": "SPEC-a1b2c3d4", "distance": 0.08}
]}
```

## Issues search output format

```json
{"ok": true, "issueResults": [
  {"number": 42, "title": "Add vector search for Issues", "url": "https://github.com/...", "state": "open", "labels": ["gwt-spec"], "distance": 0.08}
]}
```

## When to use

- Spec integration: find the canonical SPEC before creating or updating a spec
- Local spec lookup: search `specs/SPEC-*/` directories for existing local specifications
- Task start: search for Issues and SPECs related to the assigned feature
- Bug investigation: find SPECs that might relate to the bug
- Feature addition: locate relevant specs for similar implementations

## Environment

- `GWT_PROJECT_ROOT`: absolute path to the project root (set by gwt at pane launch)

## Notes

- Issue index must be updated manually (via GUI "Update Index" button or `index-issues` action)
- Uses semantic similarity (not just keyword matching)
- Lower distance values indicate higher relevance
- For file search, use `gwt-file-search` instead
- Local SPEC search via `spec_artifact.py --list-all` provides direct filesystem access without requiring an index
