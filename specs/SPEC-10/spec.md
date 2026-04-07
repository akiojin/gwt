# Project Workspace — Initialization, Clone, Migration

## Background

gwt-tui needs to handle three distinct startup scenarios:

1. **No repository** — launched in an empty or non-repo directory → guide user through cloning
2. **Existing repository** — launched in a git repo → normal workspace entry
3. **Legacy repository** — launched in a bare or legacy gwt repository → migration guidance

Currently, gwt-tui assumes a valid git repository exists and shows an empty Branches screen when no repo is found. This SPEC addresses the initialization flow, clone wizard, and workspace migration.

## User Stories

### US-1: First-time workspace setup (P0)

As a developer launching gwt-tui in a directory without a Git repository, I want to be guided through repository cloning so that I can start working immediately.

**Acceptance Scenarios:**

1. Given gwt-tui is launched in an empty directory, when the TUI starts, then an Initialization screen is displayed (fullscreen, modal, blocks other navigation).
2. Given the Initialization screen is displayed, when the user enters a repository URL and presses Enter, then a Normal Shallow Clone (`git clone --depth=1`) is executed.
3. Given the clone completes successfully, when the TUI transitions, then the workspace reloads in-place (no restart) and the Management layer is shown.
4. Given the clone fails, when the error is shown, then the user is returned to the URL input to retry.
5. Given the Initialization screen is displayed, when the user presses Esc, then gwt-tui exits.
6. Given gwt-tui is launched in a non-empty non-repo directory, then the same Initialization screen is displayed.

### US-2: Normal Clone only (P0)

As a developer, I want gwt to use Normal Clone instead of Bare Clone so that the repository structure is simpler.

**Acceptance Scenarios:**

1. Given the Clone Wizard is invoked, when the user provides a URL, then only Normal Shallow Clone is offered.
2. Given the clone target has a `develop` branch, when cloning completes, then `develop` is checked out.
3. Given the clone target does not have `develop`, when cloning completes, then the default branch is checked out.

### US-3: Existing repository detection (P0)

As a developer, I want gwt-tui to detect an existing Git repository and enter the workspace directly.

**Acceptance Scenarios:**

1. Given gwt-tui is launched inside a Git repository, when the TUI starts, then it detects the repo root and enters Management layer with Branches tab.
2. Given gwt-tui is launched inside a worktree, when the TUI starts, then it resolves to the main repo root.
3. Given the shared project-index runtime is missing or corrupted, when gwt-tui enters an existing repository, then it repairs `~/.gwt/runtime/chroma_index_runner.py` and the managed Python venv before search features are used.

### US-4: Legacy bare repo migration guidance (P1)

As a developer with a legacy bare repository, I want clear guidance on how to migrate to a normal clone.

**Acceptance Scenarios:**

1. Given gwt-tui is launched in a bare repository, when detected, then an error screen displays migration instructions.
2. The migration instructions include the command to re-clone: `git clone --depth=1 <url>`.

### US-5: develop branch commit protection (P1)

As a project maintainer, I want develop to be protected from accidental direct commits.

**Acceptance Scenarios:**

1. Given gwt clones a new repository, when clone completes, then a pre-commit hook is auto-installed blocking commits on develop.
2. Given the hook is installed, when any process attempts `git commit` on develop, then the commit is rejected with a clear error.

## Edge Cases

- Clone URL with authentication required (SSH key, token) — display git error clearly
- Repository with no branches — handle gracefully
- Interrupted clone (Ctrl+C) — clean up partial clone directory
- Pre-commit hook already exists — merge rather than overwrite
- Nested git repositories — use closest parent repo

## Functional Requirements

- **FR-001**: Detect repo type on startup: `Normal`, `Bare`, `NonRepo`. Detection scans the given path, its child directories (for `*.git` bare repos and worktree markers), and parent directories.
- **FR-002**: `ActiveLayer::Initialization` added to model — fullscreen modal, blocks Ctrl+G
- **FR-003**: Initialization screen: URL input field, clone progress, error display
- **FR-004**: Clone uses `git clone --depth=1 <url>`, attempts `-b develop` first
- **FR-005**: After successful clone, `Model::reset(new_repo_root)` reloads all state in-place
- **FR-006**: Bare repo detection shows migration error screen (not Initialization)
- **FR-007**: Pre-commit hook auto-installed after clone, blocking commits on develop/main
- **FR-008**: Hook preserves existing `.git/hooks/pre-commit` content (append, not overwrite)
- **FR-009**: Workspace initialization creates or repairs `~/.gwt/runtime/chroma_index_runner.py` from the repo-tracked runtime asset.
- **FR-010**: Workspace initialization creates or repairs the managed project-index Python venv under `~/.gwt/runtime/chroma-venv`.
- **FR-011**: Existing repository startup runs the same runtime repair path before search features load.
- **FR-012**: Runtime bootstrap failures degrade to a warning notification and do not abort TUI startup or clone completion.

## Implementation Details

### ActiveLayer Extension

```rust
pub enum ActiveLayer {
    Initialization,  // NEW: shown when no repo detected
    Main,
    Management,
}
```

### Repo Detection Flow

```
startup(path)
  ├─ path/.git exists → Normal → ActiveLayer::Management
  ├─ path/HEAD+objects+refs → Bare → migration screen
  ├─ child dir has *.git bare repo or .git worktree marker → Bare → migration screen
  ├─ child dir has .git/ directory → Normal → ActiveLayer::Management
  ├─ parent dir has .git → Normal → ActiveLayer::Management
  └─ none found → NonRepo → ActiveLayer::Initialization (clone wizard)
```

This means launching gwt in a bare repo workspace directory (e.g., `/path/gwt/` containing `gwt.git/` + `develop/` + `feature/`) correctly detects the Bare layout.

### Clone Command

```bash
# Clone into current directory (must be empty)
git clone --depth=1 -b develop <url> .
# fallback if develop doesn't exist:
git clone --depth=1 <url> .
```

### Worktree Location

Worktrees are created at the same directory level as the repository, using the
branch name itself as the relative directory hierarchy:
```
/home/user/projects/
├── my-repo/            ← main clone (develop checked out)
├── feature/
│   ├── feature-1/      ← git worktree for feature/feature-1
│   └── feature-2/      ← git worktree for feature/feature-2
└── bugfix/
    └── bugfix-1/       ← git worktree for bugfix/bugfix-1
```

### Full Initialization (on first launch in repo)

1. `~/.gwt/config.toml` — create with defaults if not exists
2. `.git/hooks/pre-commit` — install develop/main commit protection
3. `specs/` — create directory if not exists
4. `~/.gwt/runtime/chroma_index_runner.py` — write the repo-tracked project-index runner if missing or outdated
5. `~/.gwt/runtime/chroma-venv/` — create or repair the managed Python venv used for project index operations
6. Skill embedding — deferred to agent launch (per SPEC-1438 lifecycle)

### Project Index Runtime Lifecycle

- Runtime assets live in the repo and are copied into `~/.gwt/runtime/` during workspace initialization and normal startup repair.
- The managed venv lives at `~/.gwt/runtime/chroma-venv/` for backward compatibility with existing skills and user environments.
- If the venv is missing, lacks `chromadb`, or fails the import probe, gwt rebuilds it once before surfacing a warning.
- Startup and clone completion continue even when runtime repair fails; the user sees a warning instead of an app crash.

### Skill Embedding Lifecycle (per SPEC-1438)

Skills are embedded on **every agent launch**, NOT during workspace initialization:
- Claude Code: `.claude/skills/`, `.claude/commands/`, `.claude/settings.local.json`
- Codex: `.codex/skills/`, `.codex/hooks.json`
- Gemini: `.gemini/skills/`
- Registration/repair runs on each launch targeting the worktree root
- Generated files excluded via `.git/info/exclude`

### Pre-commit Hook

```bash
#!/bin/sh
# gwt-managed: protect develop and main branches
branch=$(git symbolic-ref --short HEAD 2>/dev/null)
if [ "$branch" = "develop" ] || [ "$branch" = "main" ]; then
  echo "ERROR: Direct commits to $branch are blocked by gwt."
  echo "Create a feature branch: git checkout -b feature/<name>"
  exit 1
fi
```

### Keybindings (Initialization Screen)

| Keybinding | Action |
|------------|--------|
| `Enter` | Start clone |
| `Esc` | Exit gwt-tui |
| Typing | URL input |
| `Backspace` | Delete character |

## Non-Functional Requirements

- **NFR-001**: Clone shows progress indication
- **NFR-002**: Model reset after clone completes within 2 seconds
- **NFR-003**: Pre-commit hook does not interfere with existing hooks
- **NFR-004**: Runtime repair is idempotent; re-running startup without asset drift does not reinstall dependencies.

## Success Criteria

- **SC-001**: gwt-tui in empty directory shows Initialization screen, not empty Branches
- **SC-002**: Clone Wizard offers only Normal Shallow Clone
- **SC-003**: After clone, TUI transitions to Management layer without restart
- **SC-004**: `git commit` on develop is blocked by pre-commit hook
- **SC-005**: Bare repo shows migration instructions
- **SC-006**: Removing `~/.gwt/runtime/chroma_index_runner.py` and restarting gwt recreates the runner automatically.
- **SC-007**: Removing or corrupting `~/.gwt/runtime/chroma-venv` and restarting gwt repairs the managed venv automatically.
