# Data Model: SPEC-9 - Infrastructure

## Primary Entities
### DockerProgressState
- Role: Tracks service setup progress and user-visible Docker status.
- Invariant: Displayed progress must map to real backend lifecycle events.

### EmbeddedSkillManifest
- Role: Describes built-in skill packaging and availability.
- Invariant: Manifest state must stay synchronized with bundled assets.

### HooksMergePlan
- Role: Defines safe merge, backup, and restore handling for git hooks.
- Invariant: Backup and recovery paths must not lose user hooks.

### ManagedRuntimeHookEntry (US-10)
- Role: Canonical shape of a gwt-managed runtime-state entry inside `.claude/settings.local.json` / `.codex/hooks.json`.
- Fields:
  - `type`: always `"command"`.
  - `command`: exactly `node <worktree>/<scripts>/gwt-runtime-state.mjs <event>` where `<scripts>` is `.claude/hooks/scripts` or `.codex/hooks/scripts` and `<event>` is one of `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Stop`.
  - `env.GWT_MANAGED_HOOK`: `"runtime-state"` when the hook schema supports `env`, otherwise the marker is repeated inside the command string for legacy-compatible detection.
- Invariants:
  - Byte-identical on POSIX and Windows for the same `<worktree>` path (no platform-conditional fallback).
  - Never references `sh -lc`, `powershell`, `gwt`, `gwt-tauri`, or `gwt-forward-hook.mjs`.
  - Exists exactly once per event in the gwt-managed block.

### BashGuardHookEntry (US-9)
- Role: PreToolUse `Bash` matcher entries that reject worktree-escape patterns before the agent executes.
- Required members (ordered):
  1. `node <worktree>/<scripts>/gwt-block-git-branch-ops.mjs` — blocks destructive `git` branch/worktree operations and interactive rebase against `origin/main`.
  2. `node <worktree>/<scripts>/gwt-block-cd-command.mjs` — blocks `cd` targets outside the worktree root (all segments evaluated).
  3. `node <worktree>/<scripts>/gwt-block-file-ops.mjs` — blocks `mkdir`/`rmdir`/`rm`/`touch`/`cp`/`mv` operands outside the worktree root.
  4. `node <worktree>/<scripts>/gwt-block-git-dir-override.mjs` — blocks `GIT_DIR` / `GIT_WORK_TREE` environment overrides.
- Invariants:
  - Ordering matches FR-050 (git-branch → cd → file-ops → git-dir).
  - Each entry exits `2` with a JSON `{decision:"block", reason, stopReason}` body on block, and `0` on permit.
  - None of the four scripts spawn a secondary `gwt`/`gwt-tauri` process.

### LegacyRuntimeHookShape (US-10 migration)
- Role: Set of historical command shapes that legacy detection must recognize so migration fires.
- Members:
  - `LegacyForwarder`: command contains `"gwt-forward-hook.mjs"`.
  - `LegacyPosixShell`: command matches `GWT_MANAGED_HOOK=runtime-state ... sh -lc '...'`.
  - `LegacyPowerShell`: command matches `powershell -NoProfile -Command "..." ... GWT_MANAGED_HOOK ... runtime-state ...`.
- Invariant: detection returns true if ANY gwt-managed entry in the file matches ANY member. The migration writer replaces only matched entries with `ManagedRuntimeHookEntry` and leaves user hooks untouched.

## Lifecycle Notes
- `metadata.json`, `tasks.md`, and `progress.md` must stay aligned.
- Completion cannot be claimed from implementation alone; the checklists must agree.
- `ManagedRuntimeHookEntry` and `BashGuardHookEntry` are both emitted by the same typed builder in `crates/gwt-skills/src/settings_local.rs`; changes to one must preserve the ordering invariants of the other.
