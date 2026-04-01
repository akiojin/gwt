# Research: SPEC-1787

## Decision Log

### D1: Normal Clone vs Bare Clone

**Decision**: Normal Clone only. Bare abolished.

**Rationale**: Bare Clone requires worktree for every operation. Normal Clone provides a working directory immediately (develop checkout). `git worktree add` works identically on Normal repos for parallel branch work. No functional loss.

**Impact**: Remove ~200 lines of Bare-specific code (bare_project.rs, Sibling strategy, Bare detection, migration dialog).

### D2: Clone depth

**Decision**: `--depth=1` (Shallow Clone).

**Rationale**: Faster initial clone. Full history can be fetched later with `git fetch --unshallow` if needed. worktree operations work on shallow clones.

### D3: develop branch preference

**Decision**: `git clone --depth=1 -b develop <url>`. Fallback to default branch if develop doesn't exist.

**Rationale**: gwt workflow uses develop as the integration branch. Feature branches are created from develop. If a repo doesn't have develop, the default branch (usually main) serves the same purpose.

**Implementation**: Try clone with `-b develop` first. If git returns error (branch not found), retry without `-b develop`.

### D4: In-place repo_root switch vs TUI restart

**Decision**: In-place switch via `Model::reset()`.

**Rationale**: Seamless UX. User sees clone progress → SPECs tab without interruption. TUI restart would flash the terminal and lose context.

### D5: Pre-commit hook implementation

**Decision**: Bash script in `.git/hooks/pre-commit` that checks current branch.

**Rationale**: Simplest approach. Works with all git clients and agents. No external dependencies.

**Merge strategy**: If a pre-commit hook already exists, append the gwt guard section with a marker comment (`# gwt-develop-guard-start` / `# gwt-develop-guard-end`).

### D6: SPEC drafting workflow

**Decision**: Agent launched on develop via existing PTY mechanism. New SPEC drafting skill provides instructions.

**Rationale**: Reuses existing agent launch infrastructure. No new TUI components needed beyond a wizard mode and skill file.

### D7: Existing Bare repository handling

**Decision**: Show error message with re-clone instructions. No automatic migration.

**Rationale**: Automatic migration (Bare → Normal) is complex and error-prone. Users can re-clone in minutes. Breaking change is acceptable for a pre-1.0 tool.

## Tradeoffs

| Tradeoff | Chosen Side | Alternative | Why |
|----------|-------------|-------------|-----|
| Shallow vs Full clone | Shallow | Full | Speed > history access on first use |
| Hook vs CI-only protection | Hook + AGENTS.md | CI-only | Hook catches errors locally before push |
| Error msg vs migration wizard | Error msg | Migration wizard | Simpler; re-clone is fast |
| In-place switch vs restart | In-place | Restart | Better UX, moderate implementation cost |
