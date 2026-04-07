# Data Model: SPEC-10 - Project Workspace

## Primary Entities
### WorkspaceBootstrapRequest
- Role: User request to open, clone, or initialize a project workspace.
- Invariant: Bootstrap flow must choose exactly one acquisition path.

### RepositoryKind
- Role: Classification of local repository state such as bare or working tree.
- Invariant: Kind detection must drive migration behavior deterministically.

### MigrationPlan
- Role: Safe conversion steps for adopting older repository layouts.
- Invariant: Migration must preserve repository data before switching modes.

## Index Lifecycle Entities (Phase 8)

### RepoHash
- Role: Stable identifier for a repository, derived from `git remote get-url origin`.
- Computation: SHA256 of the normalized URL, truncated to the first 16 hex characters.
- Normalization: lowercase host and path, strip `.git` suffix, unify `git@host:path` and `https://host/path` forms to `host/path`.
- Invariant: Two checkouts of the same upstream repository (HTTPS clone vs SSH clone vs second worktree) must produce the same `repo_hash`.

### WorktreeHash
- Role: Stable identifier for a Worktree directory.
- Computation: SHA256 of the canonicalized absolute path of the Worktree, truncated to the first 16 hex characters.
- Invariant: Symlinked paths and direct paths to the same on-disk directory produce the same hash.

### Scope
- Role: Enumeration of index categories.
- Variants: `Issues`, `Specs`, `FilesCode`, `FilesDocs`.
- Invariant: `Issues` requires only `repo_hash`; the other variants require both `repo_hash` and `worktree_hash`.

### IndexManifest
- Role: Source of truth for incremental indexing of SPEC and File scopes.
- File: `~/.gwt/index/<repo-hash>/worktrees/<wt-hash>/manifest.json`
- Schema:
  ```json
  {
    "schema_version": 1,
    "scope": "files",
    "entries": [
      {"path": "src/lib.rs", "mtime": 1712345678, "size": 4096}
    ]
  }
  ```
- Diff rule: An entry is considered "changed" if either `mtime` or `size` differs from the recorded tuple.

### RepoMetadata
- Role: Repo-level index metadata.
- File: `~/.gwt/index/<repo-hash>/meta.json`
- Schema:
  ```json
  {
    "schema_version": 1,
    "repo_url": "github.com/akiojin/gwt",
    "created_at": "2026-04-07T12:00:00Z"
  }
  ```

### IssueIndexMetadata
- Role: Tracks Issue index TTL state.
- File: `~/.gwt/index/<repo-hash>/issues/meta.json`
- Schema:
  ```json
  {
    "schema_version": 1,
    "last_full_refresh": "2026-04-07T12:00:00Z",
    "ttl_minutes": 15
  }
  ```
- TTL: A refresh job is skipped (when `--respect-ttl` is set) if `now - last_full_refresh < ttl_minutes`.

### WorktreeIndexMetadata
- Role: Worktree-level index metadata.
- File: `~/.gwt/index/<repo-hash>/worktrees/<wt-hash>/meta.json`
- Schema:
  ```json
  {
    "schema_version": 1,
    "worktree_path": "/Users/akiojin/Workbench/gwt/feature/foo",
    "branch": "feature/foo",
    "created_at": "2026-04-07T12:00:00Z"
  }
  ```

### LockSentinel
- Role: Cross-process mutual-exclusion file for ChromaDB writers/readers.
- File: `<db-dir>/.lock` (one per index DB directory; e.g., `<repo>/issues/.lock`, `<repo>/worktrees/<wt>/.lock`)
- Mode: Exclusive for `index-*`, shared for `search-*`. Released on context-manager exit even on exception.

### WatcherBatch
- Role: A debounced collection of filesystem events to feed `runner index-* --mode incremental`.
- Invariant: Each batch contains at most 100 paths and is emitted no sooner than 2 seconds after the most recent event.

## Lifecycle Notes
- `metadata.json`, `tasks.md`, and `progress.md` must stay aligned.
- Completion cannot be claimed from implementation alone; the checklists must agree.
- `RepoHash` and `WorktreeHash` are stable identifiers — once a directory is created under either, it must not be reassigned to a different upstream/path without GC.
