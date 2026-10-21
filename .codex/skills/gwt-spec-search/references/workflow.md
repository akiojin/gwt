# SPEC Search

gwt indexes SPECs once as heading chunks in the same repo-scoped `issues`
generation store as plain Issues, using ChromaDB and
`intfloat/multilingual-e5-base`. The `specs` scope filters this shared collection
to `gwt-spec` labeled Issues cached at `~/.gwt/cache/issues/`. The first rebuild
of either scope upgrades the old layout from the cache; old separate SPEC
stores are no longer read. Use JSON operation `issue.spec.pull` with
`{"all":true}` to refresh the cache before searching. Existing commands and
SPEC result fields remain compatible. Use `scopes:["specs","issues"]` for
both kinds, or omit scopes to include all searchable content.

## gwtd resolution

Before executing any `gwtd ...` command from this skill or its references,
resolve `GWT_BIN` first: executable `GWT_BIN_PATH`, then `command -v gwtd`,
then `$GWT_PROJECT_ROOT/target/debug/gwtd` or `./target/debug/gwtd`. Run the
command as `"$GWT_BIN" ...`; if none exists, stop with an actionable
`gwtd not found` error.

## SPEC search first for spec integration

Use SPEC search when the question needs discovery across existing specifications. Direct artifact reads or exact searches are appropriate for known candidates:

- "既存仕様を探して"
- "どの SPEC に統合するべきか"
- "関連する SPEC を探して"
- "この機能の仕様は？"
- "重複する SPEC はないか確認して"

Suggested workflow:

1. Run `search` JSON envelopes with `params.scopes:["specs"]` and query variations needed to establish owner coverage
2. Pick the canonical existing spec if found
3. Only fall back to creating a new spec when no suitable canonical spec exists

## SPEC search command

The `search` JSON operation is the canonical gwtd entry point (SPEC-1942
US-15). Run it from inside the target worktree; the repo is resolved from the
current directory.

```bash
"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"search","params":{"query":"your search query","scopes":["specs"],"n_results":10}}
JSON
```

If the SPEC index does not yet exist, the search builds it automatically from the repo-scoped Issue cache before returning results (the first call may take longer).

To force a full re-index (normally handled by the watcher / auto-build):

```bash
"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"index.rebuild","params":{"scope":"specs"}}
JSON
```

## Output format

```json
{"ok": true, "query": "...", "results": [
  {"scope": "specs", "title": "SPEC-1939: Semantic search platform", "subtitle": "open · phase/review", "preview": "...", "distance": 0.08, "target": {"kind": "spec", "spec_id": 1939}}
], "suggestions": []}
```

## When to use

- Spec integration: find the canonical spec before creating or updating
- Task start: search for specs related to the assigned feature
- Duplicate check: verify no existing spec covers the same scope
- Architecture understanding: discover how features are specified

## Notes

- The search refreshes the repo-scoped SPEC index from the repo-scoped Issue cache when invoked outside the GUI
- A missing index is auto-built on the first search
- An `EMPTY_CORPUS` error means the Issue cache is unpopulated — refresh the cache with JSON operation `issue.spec.pull` and retry; do **not** conclude that no SPEC owner exists
- Uses semantic similarity (not just keyword matching)
- Lower distance values indicate higher relevance
- For file search, use `gwt-project-search` instead
- For GitHub Issue search, use `gwt-issue-search` instead

## Fallback: direct runner invocation (older binaries only)

Only when the `search` JSON operation is unavailable in an older gwtd binary,
call the Python runner directly:

```bash
~/.gwt/runtime/chroma-venv/bin/python3 ~/.gwt/runtime/chroma_index_runner.py \
  --action search-specs \
  --repo-hash "$GWT_REPO_HASH" \
  --worktree-hash "$GWT_WORKTREE_HASH" \
  --project-root "$GWT_PROJECT_ROOT" \
  --query "your search query" \
  --n-results 10
```

On Windows, use `~/.gwt/runtime/chroma-venv/Scripts/python.exe`. The hashes
are an optimization, not a requirement: when `GWT_REPO_HASH` and
`GWT_WORKTREE_HASH` are unset or passed empty, the runner derives them from
`--project-root` automatically (Issue #2933). The fallback returns the legacy
`{"ok": true, "specResults": [...]}` shape.
