# Research: SPEC-10 - Project Workspace

## Scope Snapshot
- Canonical scope: Workspace initialization, repository clone, existing-repo import, and repository migration behavior.
- Current status: `open` / `Ready for Dev`.
- Task progress: `31/33` checked in `tasks.md`.
- Notes: Implementation is almost complete, but the supporting artifact set had not been expanded beyond the core four files.

## Decisions
- Keep initialization, clone, migration, and repository-kind detection together as one workspace bootstrap flow.
- Document the high completion level without claiming the final coverage and manual tasks are done.
- Use the supporting artifacts to make the remaining completion-gate work explicit.

## Open Questions
- Confirm what evidence satisfies the last coverage-related task before the SPEC can be closed.
- Decide whether any manual repository-migration scenarios still need explicit capture in the quickstart.

## Phase 8 Research (Index Lifecycle Redesign)

### Embedding model: `intfloat/multilingual-e5-base`

- **Why not the existing `all-MiniLM-L6-v2`?** MiniLM is English-specialized; the gwt repository's SPEC files, conversations, and many code comments are Japanese. Empirically, semantic recall on Japanese SPEC titles drops below 30 % at top-5.
- **Why e5-base over MiniLM-L12 / e5-small / bge-m3?**
  - `e5-base` (~440 MB) hits a sweet spot: solid multilingual recall, ~3-5× slower inference than MiniLM but still acceptable for batch indexing on a developer laptop.
  - `bge-m3` (~2 GB) has the best benchmark numbers but the model footprint and inference cost are excessive for the spec scale (sub-1000 docs).
  - `e5-small` is too lossy for the long-form Japanese narrative in `spec.md`.
- **Prefix requirement:** All e5 family models require a `passage:` prefix for documents and a `query:` prefix for queries. Omitting the prefix degrades recall by ~30-40 percentage points. The runner injects this transparently via a custom `EmbeddingFunction` so callers never deal with it.
- **First-run cost:** ~440 MB download to `~/.cache/huggingface/`. Subsequent runs reuse the cache. The TUI bootstrap will surface a one-time warning notification during the initial download.

### `notify` vs `notify-debouncer-mini`

- Raw `notify` emits one event per fs syscall, which is too noisy for ChromaDB embedding.
- `notify-debouncer-mini` collapses bursts within a fixed debounce window into a single event set, which matches the agreed 2-second window exactly.
- Decision: use `notify-debouncer-mini` with a 2-second `Duration` and a custom batch-size cap (100) implemented on top of the debouncer's batch output.

### ChromaDB concurrent access constraints

- ChromaDB's `PersistentClient` uses sqlite under the hood. sqlite supports multi-reader / single-writer with WAL mode, but the Python `chromadb` package does not document cross-process safety guarantees, and the existence of `.gwt/index.crashed-*` directories in production suggests writers can collide.
- We therefore impose an explicit `flock`-based mutex (sentinel file in the DB directory). This is independent of sqlite's own locking, layered on top.
- Writer holds `LOCK_EX`, reader holds `LOCK_SH`. Auto-build inner step holds `LOCK_EX` then downgrades.

### `flock` cross-platform strategy

- Python: `portalocker` is the de facto cross-platform wrapper that handles fcntl on POSIX and `LockFileEx` on Windows.
- Rust: `fs2` provides the same abstraction. `fs2::FileExt::lock_exclusive` / `lock_shared`.
- Both libraries support timeouts and exception-safe release via context managers / RAII guards.

### Per-Worktree DB directories vs. shared DB

- Considered: a single ChromaDB collection with a `worktree_id` metadata filter.
- Rejected: ChromaDB's metadata filtering is significantly slower than collection-level isolation, and locking would have to be coarse-grained across all worktrees.
- Per-worktree directories are isolated, locks are scoped, and worktree-remove cleanup is `rm -rf`.

### TUI external session strategy

- The user explicitly accepted the trade-off that TUI-external sessions (e.g., a developer running `claude` directly without `gwt`) will not have a watcher.
- Mitigation: the runner's `search-*` actions perform an mtime + size scan on every invocation when a manifest exists, picking up changes since the last run before searching. This is slower than a watcher but always correct.
- This means a long-lived non-TUI session will incur per-search reconciliation cost, but never returns stale results.
