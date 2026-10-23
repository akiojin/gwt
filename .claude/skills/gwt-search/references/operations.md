# Unified Search

gwt maintains ChromaDB vector search indexes for the scopes below (Phase 8
layout plus SPEC-2805 Memory):

| Scope | Content | Lifecycle |
|-------|---------|-----------|
| SPECs | GitHub Issue cache (`~/.gwt/cache/issues/`) | Populated by JSON operation `issue.spec.pull` or gwt GUI startup sync |
| Issues | GitHub Issues (all states) | gwt GUI startup async refresh (TTL 15 min) + auto-build on first search |
| Files | Project implementation files (excludes skill assets, SPEC trees, snapshots) | Per-worktree watcher (gwt GUI) + auto-build on first search |
| Project docs | Project documentation files | Per-worktree watcher (gwt GUI) + auto-build on first search |
| Memory | Post-mortem entries in the machine-local work-notes memory log (`~/.gwt/projects/<repo-hash>/work-notes/memory.md`; repo-local `.gwt/work/memory.md` is a read fallback) | Manifest refresh + auto-build on first search |
| Board | Coordination Board entries | Repo-local event log + auto-build on first search |
| Discussions | Git-managed discussion notes | Repo-local discussion notes + auto-build on first search |
| Reports | Durable PM digest and Q&A reports, including their question, answer, and Issue references | Report-ledger mtime+size refresh + auto-build on first search |

SPECs and plain Issues share the repo-scoped `issues` generation store. SPECs
appear once as heading chunks; they are excluded from plain Issue records.
The existing `specs` and `issues` scopes filter that same collection by kind.
Omitting scopes includes both kinds (and the other scopes); use
`["specs","issues"]` to search only Work Items. Result fields stay compatible.
The first rebuild of either scope upgrades the old Issue layout from the cache.
Old separate SPEC stores are no longer read; no manual deletion is required.

All vector data is stored under `~/.gwt/index/<repo-hash>/...`. Issues,
SPECs, Memory, and Reports are repo-scoped and shared across worktrees; Files
(code + docs) is worktree-scoped under `worktrees/<worktree-hash>/`. The legacy
`$WORKTREE/.gwt/index/` location is no longer used and is deleted
automatically by the gwt GUI on startup.

When invoked outside the gwt GUI app, the search falls back to a
synchronous mtime+size diff per call: results are always correct, just
slower than the GUI watcher path.

## gwtd resolution

Before executing any `gwtd ...` command from this skill or its references,
resolve `GWT_BIN` first: executable `GWT_BIN_PATH`, then `command -v gwtd`,
then `$GWT_PROJECT_ROOT/target/debug/gwtd` or `./target/debug/gwtd`. Run the
command as `"$GWT_BIN" ...`; if none exists, stop with an actionable
`gwtd not found` error.

### gwtd bootstrap order

The `search` operation is read-only, so it never waits for a checkout
build or a verification lease.

In a checkout that builds gwtd from source (the gwt repository itself), the
first `cargo build -p gwt --bin gwtd` is a lease-free bootstrap step, never a
heavy verification command. The order is build → `verify.plan` → `verify.run`:
build the checkout binary without holding or waiting for any lease, and only
then run canonical verification through it.

Decide first whether the checkout binary is needed. Only operations that
execute checkout code need it: `execution.*`, `workspace.*`, `build.*`,
`verify.*`, and any operation added in the checkout. Read-only `issue.*`,
`pr.*`, `board.*`, and `search` operations run through the resolved installed
gwtd (`GWT_BIN_PATH` / PATH). Never wait for the build or a lease just to read
Issue, PR, or Board state.

## Quick Reference

`gwt-search` is a skill, not a PATH executable. Never resolve it with
`command -v` or `Get-Command` — the lookup finds nothing by design, and
an empty lookup does not mean the tooling is missing. The executable
entrypoint is a `search` JSON operation sent to the resolved `GWT_BIN`
(SPEC-1942 US-15). Run it from inside the target worktree; the repo is
resolved from the current directory.

```bash
"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"search","params":{"query":"query","n_results":10}}
JSON

"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"search","params":{"query":"query","scopes":["specs"],"n_results":10}}
JSON
```

## Filter Parameters

| JSON parameter | Scope |
|----------------|-------|
| omit `params.scopes` | All scopes merged (same default as the GUI search window) |
| `"scopes":["specs"]` | SPEC Issues only |
| `"scopes":["issues"]` | Plain GitHub Issues only (excludes `gwt-spec`) |
| `"scopes":["files"]` | Implementation files only |
| `"scopes":["files_docs"]` | Project docs only |
| `"scopes":["memory"]` | Post-mortem memory only |
| `"scopes":["board"]` | Coordination Board entries only |
| `"scopes":["discussions"]` | Git-managed discussion notes only |
| `"scopes":["reports"]` | Durable PM digest and Q&A reports only |

Scopes merge by listing multiple values, such as
`"scopes":["issues","specs"]`. `params.n_results` limits the result count,
and `params.match_mode:"semantic"|"all_terms"` selects the match mode.

## Match modes

Use the default semantic mode for broad discovery. Use `params.match_mode:"all_terms"`
when the user or task needs FAQ-style precision and every whitespace-separated
term or quoted phrase must be present in a strict result.

Examples:

```bash
"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"search","params":{"query":"Workspace 置き換え","match_mode":"all_terms","n_results":10}}
JSON

"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"search","params":{"query":"\"Project State\" migration","match_mode":"all_terms","n_results":10}}
JSON
```

In `all_terms` mode, strict results must satisfy every required term. Semantic
suggestions may still be returned separately (in the `suggestions` array), but
they must not be treated as strict matches.
