## Fallback: direct runner invocation (older binaries only)

Only when the `search` JSON operation is unavailable in an older gwtd binary,
call the embedded Python runner directly:

```bash
PYTHON=~/.gwt/runtime/chroma-venv/bin/python3   # Windows: ~/.gwt/runtime/chroma-venv/Scripts/python.exe
RUNNER=~/.gwt/runtime/chroma_index_runner.py

$PYTHON $RUNNER \
  --action search-issues \
  --repo-hash "$GWT_REPO_HASH" \
  --project-root "$GWT_PROJECT_ROOT" \
  --query "your search query" \
  --n-results 10
```

Scope actions: `search-specs` / `search-issues` / `search-files` /
`search-files-docs` / `search-memory`. `search-specs`, `search-files`, and
`search-files-docs` additionally take `--worktree-hash "$GWT_WORKTREE_HASH"`.
`--match-mode all_terms` is accepted by every search action.

The hashes are an optimization, not a requirement: when `GWT_REPO_HASH` /
`GWT_WORKTREE_HASH` are unset or passed empty (e.g. when the launch
environment did not export them), the runner derives them from
`--project-root` automatically (Issue #2933) — no manual hash recomputation,
and no dependency on `sha256sum` (absent on stock macOS).

The fallback returns legacy per-scope shapes (`specResults` / `issueResults` /
`results` / `memoryResults`) instead of the unified `results` array, and
emits NDJSON auto-build progress on stderr. Pass `--no-auto-build` to disable
inline index builds; the runner then returns
`{"ok": false, "error_code": "INDEX_MISSING", ...}`.
