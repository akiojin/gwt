# Contract: chroma_index_runner.py CLI (Phase 8)

This document is the canonical interface for `crates/gwt-core/runtime/chroma_index_runner.py` after the Phase 8 redesign. Both the gwt TUI process and skill-driven LLM/agent invocations conform to this contract.

## Invocation form

```bash
python3 chroma_index_runner.py \
  --action <action> \
  --repo-hash <hex16> \
  [--worktree-hash <hex16>] \
  [--scope <issues|specs|files|files-docs>] \
  [--query <text>] \
  [--n-results <int>] \
  [--mode <full|incremental>] \
  [--no-auto-build] \
  [--respect-ttl] \
  [--project-root <path>]
```

## Actions

| Action | Required args | Optional args | Effect |
|---|---|---|---|
| `index-issues` | `--repo-hash`, `--project-root` | `--respect-ttl` | Refreshes Issues collection. Updates `<issues>/meta.json::last_full_refresh`. With `--respect-ttl`, no-op if within TTL. |
| `index-specs` | `--repo-hash`, `--worktree-hash`, `--project-root` | `--mode` (default `full`) | Indexes `specs/SPEC-*` of the worktree. `incremental` consults `manifest.json` and re-embeds only changed files. |
| `index-files` | `--repo-hash`, `--worktree-hash`, `--project-root` | `--mode`, `--scope {files,files-docs}` | Same as above for code/docs. |
| `search-issues` | `--repo-hash`, `--query` | `--n-results` (default 10), `--no-auto-build` | If index missing, auto-builds (unless suppressed) then searches. |
| `search-specs` | `--repo-hash`, `--worktree-hash`, `--query` | `--n-results`, `--no-auto-build` | Same as above. |
| `search-files` | `--repo-hash`, `--worktree-hash`, `--query` | `--n-results`, `--no-auto-build`, `--scope` | Same as above. Default scope is `files` (code). |
| `search-files-docs` | `--repo-hash`, `--worktree-hash`, `--query` | `--n-results`, `--no-auto-build` | Searches docs collection. |
| `status` | `--repo-hash` | `--worktree-hash`, `--scope` | Returns metadata snapshot incl. `ttl_remaining_seconds` for issues. |

## DB path resolution

Computed internally; clients never specify `--db-path` directly:

```
~/.gwt/index/<repo-hash>/issues/                                  # scope=issues
~/.gwt/index/<repo-hash>/worktrees/<wt-hash>/specs/               # scope=specs
~/.gwt/index/<repo-hash>/worktrees/<wt-hash>/files/               # scope=files
~/.gwt/index/<repo-hash>/worktrees/<wt-hash>/files-docs/          # scope=files-docs
```

## stdout: result protocol

Single JSON object on stdout, terminated by newline.

### Success — index actions

```json
{"ok": true, "indexed": 850, "elapsed_ms": 12340, "scope": "files"}
```

### Success — search actions

```json
{"ok": true, "results": [
  {"path": "src/git/issue.rs", "description": "...", "distance": 0.12}
]}
```

For Issues:

```json
{"ok": true, "issueResults": [
  {"number": 42, "title": "...", "url": "...", "state": "open", "labels": ["enhancement"], "distance": 0.08}
]}
```

For SPECs:

```json
{"ok": true, "specResults": [
  {"spec_id": "10", "title": "...", "status": "in-progress", "phase": "Implementation", "dir_name": "SPEC-10", "distance": 0.08}
]}
```

### Status

```json
{
  "ok": true,
  "status": {
    "issues": {
      "exists": true,
      "last_full_refresh": "2026-04-07T12:00:00Z",
      "ttl_minutes": 15,
      "ttl_remaining_seconds": 612
    }
  }
}
```

### Error

```json
{"ok": false, "error_code": "INDEX_MISSING", "error": "human readable message"}
```

Error codes:
- `INDEX_MISSING` — only emitted when `--no-auto-build` is set and the index is absent.
- `INDEX_BUILDING` — only emitted when a concurrent writer holds the exclusive lock and the call is non-blocking (currently unused; reserved for future).
- `LOCK_TIMEOUT` — flock acquisition timed out.
- `BAD_ARGS` — required argument missing or scope/action mismatch.
- `RUNTIME_ERROR` — uncaught exception, includes traceback summary in `error`.

## stderr: progress protocol

NDJSON, one event per line. Clients may parse or ignore.

```json
{"phase":"indexing","scope":"files","done":120,"total":850}
{"phase":"embedding","scope":"files","done":120,"total":850}
{"phase":"writing","scope":"files","done":120,"total":850}
```

A final line is emitted when the action completes:

```json
{"phase":"complete","scope":"files","total":850,"elapsed_ms":12340}
```

## Locking semantics

- Each DB directory contains a `.lock` sentinel file.
- `index-*` actions acquire `LOCK_EX`.
- `search-*` actions (including auto-build inner step) acquire `LOCK_EX` during the inner build, then downgrade to `LOCK_SH` for the search.
- Locks are released on context-manager exit even on exception/panic.

## Embedding model

- `intfloat/multilingual-e5-base` via `sentence-transformers`.
- Documents are passed as `"passage: " + text` to the encoder.
- Queries are passed as `"query: " + text`.
- Re-indexing detects existing prefixes to avoid double application.
- First-run downloads model weights to `~/.cache/huggingface/` (~440 MB).

## Backwards compatibility

The pre-Phase-8 `--db-path` argument is removed. There is no migration path; legacy `$WORKTREE/.gwt/index/` directories are silently deleted by the TUI startup reconcile job (FR-027).
