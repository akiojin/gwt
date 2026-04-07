# Contract: Index Path Layout (Phase 8)

This document is the canonical specification for the on-disk layout of vector index data after the Phase 8 redesign. Both the Python runner and the Rust `gwt-core` helpers compute paths according to this contract.

## Filesystem layout

```
~/.gwt/
└── index/
    └── <repo-hash>/
        ├── meta.json                            # RepoMetadata
        ├── issues/
        │   ├── chroma.sqlite3
        │   ├── .lock
        │   └── meta.json                        # IssueIndexMetadata
        └── worktrees/
            └── <wt-hash>/
                ├── meta.json                    # WorktreeIndexMetadata
                ├── manifest.json                # IndexManifest (combined for specs+files)
                ├── .lock
                ├── specs/
                │   └── chroma.sqlite3
                ├── files/
                │   └── chroma.sqlite3           # code collection
                └── files-docs/
                    └── chroma.sqlite3           # docs collection
```

## Hash computation

### `repo-hash`

```
input := git remote get-url origin       # e.g., https://github.com/Akiojin/gwt.git
normalized := normalize(input)
repo_hash := lower(hex(sha256(normalized)))[:16]
```

`normalize(url)`:

1. Strip surrounding whitespace.
2. If matches `git@<host>:<path>` → rewrite to `<host>/<path>`.
3. If matches `<scheme>://[user[:pass]@]<host>[:port]/<path>` → take `<host>/<path>`.
4. Strip trailing `.git` from `<path>`.
5. Lowercase the entire result.

Examples (all produce the same `repo_hash`):

- `https://github.com/akiojin/gwt.git`
- `https://github.com/Akiojin/gwt`
- `git@github.com:akiojin/gwt.git`
- `ssh://git@github.com:22/akiojin/gwt.git`

→ normalized = `github.com/akiojin/gwt`
→ `repo_hash = sha256("github.com/akiojin/gwt")[:16]`

### `worktree-hash`

```
input := canonicalize(absolute_worktree_path)   # std::fs::canonicalize on Rust, Path.resolve() on Python
worktree_hash := lower(hex(sha256(str(input))))[:16]
```

- Canonicalization resolves symlinks. Two paths pointing to the same on-disk directory produce the same hash.
- Relative paths are rejected with `BAD_ARGS`.
- Trailing slashes are stripped before hashing.

## `Scope` enum

| Variant | Required hashes | Subdirectory |
|---|---|---|
| `Issues` | `repo` only | `issues/` |
| `Specs` | `repo` + `worktree` | `worktrees/<wt>/specs/` |
| `FilesCode` | `repo` + `worktree` | `worktrees/<wt>/files/` |
| `FilesDocs` | `repo` + `worktree` | `worktrees/<wt>/files-docs/` |

## Helper API (Rust, `crates/gwt-core/src/index/paths.rs`)

```rust
pub enum Scope {
    Issues,
    Specs,
    FilesCode,
    FilesDocs,
}

pub fn gwt_index_root() -> PathBuf;                                  // ~/.gwt/index/
pub fn gwt_index_repo_dir(repo: &RepoHash) -> PathBuf;
pub fn gwt_index_db_path(
    repo: &RepoHash,
    worktree: Option<&WorktreeHash>,
    scope: Scope,
) -> PathBuf;
```

`gwt_index_db_path` panics in debug builds when `worktree` is `None` for a worktree-scoped variant. In release builds it returns `Err(BadArgs)`.

## Helper API (Python, runner-internal)

```python
def resolve_db_path(
    repo_hash: str,
    worktree_hash: Optional[str],
    scope: str,  # one of "issues", "specs", "files", "files-docs"
) -> pathlib.Path:
    ...
```

## GC rules

- TUI startup `reconcile_repo()`:
  1. For each `<repo-hash>/worktrees/<wt-hash>/` directory, if no entry of `git worktree list` canonicalizes to a path with that `wt-hash`, delete the directory.
  2. For each open worktree, scan `$WORKTREE/.gwt/index/` and delete it if present (legacy cleanup).
- Worktree-remove handler invokes `remove_worktree_index(repo, wt)` synchronously before the underlying `git worktree remove`.

## Constants

| Constant | Value |
|---|---|
| `INDEX_SCHEMA_VERSION` | `1` |
| `MANIFEST_FILENAME` | `manifest.json` |
| `LOCK_FILENAME` | `.lock` |
| `META_FILENAME` | `meta.json` |
| `ISSUE_TTL_MINUTES_DEFAULT` | `15` |
| `WATCHER_DEBOUNCE_SECS` | `2` |
| `WATCHER_BATCH_LIMIT` | `100` |
| `HASH_HEX_LEN` | `16` |
