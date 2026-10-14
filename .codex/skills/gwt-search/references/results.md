## Output format

```json
{"ok": true, "query": "...", "results": [
  {"scope": "issues", "title": "#42 Add vector search for Issues", "subtitle": "open", "preview": "enhancement", "distance": 0.08, "target": {"kind": "issue", "number": 42}},
  {"scope": "specs", "title": "SPEC-1939: Semantic search platform", "subtitle": "open · phase/review", "preview": "...", "distance": 0.09, "target": {"kind": "spec", "spec_id": 1939}},
  {"scope": "files", "title": "src/git/issue.rs", "subtitle": "GitHub Issue commands", "preview": "", "distance": 0.12, "target": {"kind": "file", "path": "src/git/issue.rs"}}
], "suggestions": []}
```

- `target` is a kind-tagged locator (`issue`, `spec`, `memory`, `discussion`,
  `board`, `file`) for follow-up reads with JSON operations such as
  `issue.spec.read`, file paths, or memory headings.
- In `all_terms` mode, results may carry `matched_terms` / `missing_terms`,
  and semantic non-strict hits arrive in `suggestions`.
- The JSON envelope entrypoint always returns the machine-readable envelope
  response expected by agents.

## Interpreting results

- Lower distance values indicate higher relevance (0.0 = exact match)
- Uses semantic similarity, not just keyword matching
- Results are merged across scopes and ranked by distance
- The embedding model is `intfloat/multilingual-e5-base` (multilingual; handles Japanese)

## Auto-build fallback

When a target index does not exist, the search builds it inline (full mode)
and then returns results — the first call on a fresh checkout may take
noticeably longer. To force a rebuild explicitly:

```bash
"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"index.rebuild","params":{"scope":"all"}}
JSON

"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"index.rebuild","params":{"scope":"issues"}}
JSON
```

## Expected latency

Every `search` call starts a one-shot runner that imports and constructs the
embedding model (SPEC-1939 FR-384), so every call is cold. Expect up to 25
seconds per call; the warm remainder without the model load is held to 8
seconds, and the attempt deadline is 30 seconds. Give each call at least 30
seconds before treating it as hung, and run preflight queries one after
another. A typed error (`INDEX_NOT_READY`, `SEARCH_UNAVAILABLE`,
`INDEX_REPAIR_REQUIRED`) is an answer, not a hang: follow its `retryable` and
`recovery` fields. Slow calls show up in `perf.summary` as `route:search` and
`route:search.warm` violations.

## Empty corpus is a tooling failure, not "no results"

SPEC and Issue searches build their corpus from the GitHub Issue cache
(`~/.gwt/cache/issues/<repo-hash>/`). When that cache is empty or unpopulated,
the search fails with an `EMPTY_CORPUS` error instead of silently returning an
empty list — an empty list would read as "no existing SPEC/Issue owner" and
cause duplicate creation.

When you see `EMPTY_CORPUS`, **do not conclude that no owner exists.** Refresh
the issue cache (open the project in the gwt GUI to sync, or run
JSON operation `issue.spec.pull` with `{"all":true}`) and retry the search. Only a successful result
with an empty list from a *populated* cache means the repository genuinely has
no matching SPEC/Issue.
