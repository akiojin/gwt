---
name: gwt-issue-search
description: "Semantic search over all GitHub Issues using vector embeddings. Use when searching for existing issues, finding related issues, checking for duplicate issues, or determining which issue owns a scope. Mandatory preflight before gwt-register-issue, gwt-fix-issue, and visible SPEC routing decisions. Use when user says 'search issues', 'find related issues', 'check for duplicates', or asks which issue owns a scope."
---

# Issue Search

gwt maintains a vector search index of GitHub Issues using ChromaDB embeddings (model: `intfloat/multilingual-e5-base`). The index is stored at `~/.gwt/index/<repo-hash>/issues/` and is Worktree-independent. The gwt GUI app (WebView built with `wry + tao + axum WebSocket` and `xterm.js`) refreshes it asynchronously at startup with a 15-minute TTL; non-GUI invocations get an auto-build on the first search.

## gwtd resolution

Before executing any `gwtd ...` command from this skill or its references,
resolve `GWT_BIN` first: executable `GWT_BIN_PATH`, then `command -v gwtd`,
then `$GWT_PROJECT_ROOT/target/debug/gwtd` or `./target/debug/gwtd`. Run the
command as `"$GWT_BIN" ...`; if none exists, stop with an actionable
`gwtd not found` error.

## Issues search first

When the user asks any of the following, use GitHub Issues search before
manual `issue.view` JSON reads, title grep, or file search:

- "既存 Issue を探して"
- "関連 Issue を探して"
- "Project Index の統合仕様を確認して"
- "bug / feature の過去設計を見たい"

Minimum workflow:

1. Run `search` JSON envelopes with `params.scopes:["issues"]` and 2-3 semantic queries derived from the request (a missing index is auto-built)
2. Pick the canonical existing issue if found
3. Only fall back to creating a new issue when no suitable canonical issue exists

Suggested query patterns:

- subsystem + purpose
  - `project index issue search spec`
- user-facing problem + architecture term
  - `chroma persisted db recovery project index`
- workflow / discoverability requirement
  - `LLM should use gwt-issue-search before spec creation`

## GitHub Issues search command

The `search` JSON operation is the canonical gwtd entry point (SPEC-1942
US-15). Run it from inside the target worktree; the repo is resolved from the
current directory.

```bash
"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"search","params":{"query":"your search query","scopes":["issues"],"n_results":10}}
JSON
```

If the Issue index does not yet exist, the search builds it inline by refreshing issue data and embedding the results before returning (the first call may take longer).

To force a refresh ignoring TTL:

```bash
"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"index.rebuild","params":{"scope":"issues"}}
JSON
```

## Output format

```json
{"ok": true, "query": "...", "results": [
  {"scope": "issues", "title": "#42 Add vector search for Issues", "subtitle": "open", "preview": "enhancement", "distance": 0.08, "target": {"kind": "issue", "number": 42}}
], "suggestions": []}
```

## When to use

- Issue lookup: find existing GitHub Issues before creating new ones
- Task start: search for Issues related to the assigned feature
- Bug investigation: find Issues that might relate to the bug
- Feature addition: locate relevant Issues for similar implementations

## Notes

- The gwt GUI app refreshes the Issue index automatically at startup (TTL 15 min); non-GUI invocations trigger an inline build on the first search
- An `EMPTY_CORPUS` error means the Issue cache is unpopulated — refresh it and retry; do **not** conclude that no owner Issue exists
- Uses semantic similarity (not just keyword matching)
- Lower distance values indicate higher relevance
- For SPEC search, use `gwt-spec-search` instead (SPECs are GitHub Issues cached at `~/.gwt/cache/issues/`)
- For file search, use `gwt-project-search` instead

## Fallback: direct runner invocation (older binaries only)

Only when the `search` JSON operation is unavailable in an older gwtd binary,
call the Python runner directly:

```bash
~/.gwt/runtime/chroma-venv/bin/python3 ~/.gwt/runtime/chroma_index_runner.py \
  --action search-issues \
  --repo-hash "$GWT_REPO_HASH" \
  --project-root "$GWT_PROJECT_ROOT" \
  --query "your search query" \
  --n-results 10
```

On Windows, use `~/.gwt/runtime/chroma-venv/Scripts/python.exe`.
`GWT_REPO_HASH` is an optimization, not a requirement: when unset or passed
empty, the runner derives it from `--project-root` automatically
(Issue #2933). The fallback returns the legacy
`{"ok": true, "issueResults": [...]}` shape.
