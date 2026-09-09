---
name: gwt-memory-search
description: "Semantic search over the project's machine-local work-notes memory log using vector embeddings. Use when looking for past post-mortem fixes, prior re-occurrence prevention notes, or before starting work that resembles an earlier failure. Use when user says 'search memory', 'find related memory', 'check past failures', 'has this been hit before', '過去 memory を引いて', '同じ失敗があるか確認して'."
---

# Memory Search

gwt maintains a vector search index over post-mortem memory entries kept in
the machine-local work-notes memory log
(`~/.gwt/projects/<repo-hash>/work-notes/memory.md`, with the repo-local
`.gwt/work/memory.md` as a read fallback) using ChromaDB embeddings (model:
`intfloat/multilingual-e5-base`). Each H2 section (`## YYYY-MM-DD — title` plus
the canonical `### 事象 / 原因 / 再発防止策` subsections) is chunked and
embedded. The index is repo-scoped and stored at
`~/.gwt/index/<repo-hash>/memory/`, shared across worktrees. Memory is the
only canonical record of post-mortem learning for this project. Use JSON
operation `memory.add` for new entries; direct edits to the memory log
are only for unusual bulk cleanup.

## gwtd resolution

Before executing any `gwtd ...` command from this skill or its references,
resolve `GWT_BIN` first: executable `GWT_BIN_PATH`, then `command -v gwtd`,
then `$GWT_PROJECT_ROOT/target/debug/gwtd` or `./target/debug/gwtd`. Run the
command as `"$GWT_BIN" ...`; if none exists, stop with an actionable
`gwtd not found` error.

## Memory search first when work resembles past failures

When the user asks any of the following, use memory search **before** writing
new code or spec text:

- "過去 memory を引いて"
- "同じ失敗があるか確認して"
- "before fixing X, check whether we have learned this before"
- "Has this regression been recorded?"

Minimum workflow:

1. Run `search` JSON envelopes with `params.scopes:["memory"]` and 2-3
   semantic queries derived from the request.
2. Pick the most relevant past memory if one exists.
3. Read the matching section in the work-notes memory log before deciding the
   approach. Reuse the existing prevention strategy when applicable.

## Memory search command

The `search` JSON operation is the canonical gwtd entry point (SPEC-1942
US-15). Run it from inside the target worktree; the repo is resolved from the
current directory.

```bash
"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"search","params":{"query":"your search query","scopes":["memory"],"n_results":10}}
JSON
```

If the memory index does not yet exist, the search builds it inline from
the work-notes memory log before returning results (the first call
may take longer).

To force a full re-index (normally handled by the project watcher or the
search auto-build fallback):

```bash
"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"index.rebuild","params":{"scope":"memory"}}
JSON
```

## Output format

```json
{"ok": true, "query": "...", "results": [
  {"scope": "memory", "title": "legacy SPEC create skipped section markers", "subtitle": "2026-05-20", "preview": "...", "distance": 0.12, "target": {"kind": "memory", "heading": "## 2026-05-20 — legacy SPEC create skipped section markers", "date": "2026-05-20"}}
], "suggestions": []}
```

When a memory spans multiple chunks (long body or paragraph-split), only the
best-scoring chunk per `(date, title)` pair is surfaced. Use the
`target.heading` field to locate the exact section in the memory log.

## When to use

- Pre-work duplication check: before fixing a bug or adding a feature, confirm
  whether a related memory already captures the prevention strategy.
- Architecture discussions: surface relevant past learnings during
  `gwt-discussion` or `gwt-arch-review`.
- Code review: cite the original memory that motivates a defensive change.
- Onboarding: discover recurring failure modes documented in the project.

## Write path

Use JSON operation `memory.add` for new reusable learning:

```bash
"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"memory.add","params":{"type":"lesson","title":"short title","context":"What happened or where the learning applies.","learning":"The reusable insight.","future_action":"What future agents should do differently."}}
JSON
```

Direct file edits remain acceptable for unusual bulk edits or manual cleanup.
The watcher and the auto-build fallback pick up either path automatically.

## Notes

- A missing memory index is auto-built on the first search.
- Uses semantic similarity (not just keyword matching). Lower distance values
  indicate higher relevance.
- For SPEC search, use `gwt-spec-search`. For GitHub Issue search, use
  `gwt-issue-search`. For implementation file search, use
  `gwt-project-search`. For a unified result across all scopes, use
  the `gwt-search` skill (omit `params.scopes` to merge every scope). These
  names are skills, not PATH executables — the executable entrypoint is a
  `search` JSON operation sent to `"$GWT_BIN"`.

## Fallback: direct runner invocation (older binaries only)

Only when the `search` JSON operation is unavailable in an older gwtd binary,
call the Python runner directly:

```bash
~/.gwt/runtime/chroma-venv/bin/python3 ~/.gwt/runtime/chroma_index_runner.py \
  --action search-memory \
  --repo-hash "$GWT_REPO_HASH" \
  --project-root "$GWT_PROJECT_ROOT" \
  --query "your search query" \
  --n-results 10
```

On Windows, use `~/.gwt/runtime/chroma-venv/Scripts/python.exe`.
`GWT_REPO_HASH` is an optimization, not a requirement: when unset or passed
empty, the runner derives it from `--project-root` automatically
(Issue #2933). `--worktree-hash` is accepted for symmetry but ignored —
memory is repo-scoped. The fallback returns the legacy
`{"ok": true, "memoryResults": [...]}` shape.
