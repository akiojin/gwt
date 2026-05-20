---
name: gwt-lessons-search
description: "Semantic search over the project's lessons log at `tasks/lessons.md` using vector embeddings. Use when looking for past post-mortem fixes, prior re-occurrence prevention notes, or before starting work that resembles an earlier failure. Use when user says 'search lessons', 'find related lesson', 'check past failures', 'has this been hit before', '過去 lesson を引いて', '同じ失敗があるか確認して'."
---

# Lessons Search

gwt maintains a vector search index over post-mortem lesson entries kept in
`tasks/lessons.md` using ChromaDB embeddings (model:
`intfloat/multilingual-e5-base`). Each H2 section (`## YYYY-MM-DD — title` plus
the canonical `### 事象 / 原因 / 再発防止策` subsections) is chunked and
embedded. The index is repo-scoped and stored at
`~/.gwt/index/<repo-hash>/lessons/`, shared across worktrees. Lessons are the
only canonical record of post-mortem learning for this project — write to
`tasks/lessons.md` directly; this skill never edits content.

## gwtd resolution

Before executing any `gwtd ...` command from this skill or its references,
resolve `GWT_BIN` first: executable `GWT_BIN_PATH`, then `command -v gwtd`,
then `$GWT_PROJECT_ROOT/target/debug/gwtd` or `./target/debug/gwtd`. Run the
command as `"$GWT_BIN" ...`; if none exists, stop with an actionable
`gwtd not found` error.

## Lessons search first when work resembles past failures

When the user asks any of the following, use lessons search **before** writing
new code or spec text:

- "過去 lesson を引いて"
- "同じ失敗があるか確認して"
- "before fixing X, check whether we have learned this before"
- "Has this regression been recorded?"

Minimum workflow:

1. Run `search-lessons` with 2-3 semantic queries derived from the request.
2. Pick the most relevant past lesson if one exists.
3. Read the matching section in `tasks/lessons.md` before deciding the
   approach. Reuse the existing prevention strategy when applicable.

## Environment

When the gwt GUI app (WebView built with `wry + tao + axum WebSocket` and
`xterm.js`) launches an agent pane, the following env vars are exported
automatically:

- `GWT_PROJECT_ROOT` — absolute path of the active worktree
- `GWT_REPO_HASH` — SHA256[:16] of the normalized origin URL
- `GWT_WORKTREE_HASH` — SHA256[:16] of the canonicalized worktree absolute path

If you invoke the runner outside the gwt app, recompute them as shown in
`gwt-search` (the runner accepts the same flags for all scopes).

## Lessons search command

```bash
~/.gwt/runtime/chroma-venv/bin/python3 ~/.gwt/runtime/chroma_index_runner.py \
  --action search-lessons \
  --repo-hash "$GWT_REPO_HASH" \
  --project-root "$GWT_PROJECT_ROOT" \
  --query "your search query" \
  --n-results 10
```

If the lessons index does not yet exist, the runner builds it inline (full
mode) from `<project_root>/tasks/lessons.md` and emits NDJSON progress on
stderr before returning the search result.

To force a full re-index (normally handled by the project watcher or the
search auto-build fallback):

```bash
~/.gwt/runtime/chroma-venv/bin/python3 ~/.gwt/runtime/chroma_index_runner.py \
  --action index-lessons \
  --repo-hash "$GWT_REPO_HASH" \
  --project-root "$GWT_PROJECT_ROOT" \
  --mode full
```

`--worktree-hash` is accepted for symmetry with the other scopes but is
ignored — lessons is repo-scoped and serves every worktree from a single
index.

## Lessons search output format

```json
{"ok": true, "lessonResults": [
  {"date": "2026-05-20", "title": "gwtd issue spec create -f は section マーカーを付けない", "heading": "## 2026-05-20 — gwtd issue spec create -f は section マーカーを付けない", "chunk_idx": 0, "distance": 0.12}
]}
```

When a lesson spans multiple chunks (long body or paragraph-split), only the
best-scoring chunk per `(date, title)` pair is surfaced. Use the `heading`
field to locate the exact section in `tasks/lessons.md`.

## When to use

- Pre-work duplication check: before fixing a bug or adding a feature, confirm
  whether a related lesson already captures the prevention strategy.
- Architecture discussions: surface relevant past learnings during
  `gwt-discussion` or `gwt-arch-review`.
- Code review: cite the original lesson that motivates a defensive change.
- Onboarding: discover recurring failure modes documented in the project.

## Write path is unchanged

This skill does **not** write to `tasks/lessons.md`. New lessons must be added
by editing the file directly with the canonical structure (`## YYYY-MM-DD —
title` + `### 事象 / ### 原因 / ### 再発防止策`). The watcher and the
auto-build fallback pick up the change automatically.

## Notes

- The runner auto-builds the lessons index when missing (use
  `--no-auto-build` to suppress).
- Uses semantic similarity (not just keyword matching). Lower distance values
  indicate higher relevance.
- For SPEC search, use `gwt-spec-search`. For GitHub Issue search, use
  `gwt-issue-search`. For implementation file search, use
  `gwt-project-search`. For a unified result across all four, use
  `gwt-search` and add `--lessons` (or omit filters to merge every scope).
