# Research: SPEC-9 - Infrastructure

## Scope Snapshot

- Canonical scope: Build distribution, Docker-oriented UI flows, embedded skills, and hooks merge behavior.
- Current status: `in-progress` / `Implementation`.
- Notes: Phase 1 (Docker), Phase 3 (Hooks), Phase 4 (Build Distribution) completed. Phase 2 (Embedded Skills) redesigned.

## Decisions

- Group Docker UI, embedded skills, release packaging, and hooks merge here because they are support infrastructure rather than core shell UX.
- Track hooks hardening separately inside progress notes so it does not hide the unfinished Docker and release work.
- Keep the quickstart focused on reviewer validation points rather than promising a fully finished infrastructure stack.
- **2026-04-04**: Embedded Skills (US-3) redesigned with three sub-phases:
  - Phase 2: Build-time bundling via `include_dir` crate. `BuiltinSkill` enum and `SKILL_CATALOG` removed.
  - Phase 2b: Runtime distribution to worktrees on agent launch. `.git/info/exclude` management and `settings.local.json` generation.
  - Phase 2c: Quality improvement per Anthropic skill authoring guidelines (third-person descriptions, Progressive Disclosure, `allowed-tools` frontmatter).
- **2026-04-04**: Decision: gwt does NOT parse YAML frontmatter at runtime. Skill files are opaque blobs bundled and written out. Parsing is Claude Code/Codex's responsibility. Build-time YAML validation only catches syntax errors.
- **2026-04-04**: Decision: Full overwrite on every agent launch (no diff-based sync). Simplest approach; user customizations in worktree-local skill files are not preserved.
- **2026-04-04**: Decision: Distribution targets `.claude/`, `.codex/`, `.agents/` simultaneously for cross-platform agent support.
- **2026-04-08**: Decision (US-9 / US-10): Unify managed runtime-state hooks behind `node .../gwt-runtime-state.mjs <event>` on both Claude and Codex, and formalize the four existing PreToolUse Bash guard hooks in the spec.
  - **Context**: Investigation of `crates/gwt-skills/src/settings_local.rs` revealed that the four `gwt-block-*.mjs` scripts are already wired via `bash_blockers_hook()` (PreToolUse/Bash matcher) as Node invocations, but have zero coverage in spec.md, tasks.md, or FRs. Only `gwt-forward-hook.mjs` had a prior spec reference (as the legacy forwarder being migrated away from).
  - **Root cause of the earlier "no-Node" rule**: The banned behavior was `gwt-forward-hook.mjs` spawning a secondary `gwt`/`gwt-tauri` process via PATH or app bundle lookup, which broke under interactive Codex sandboxing. The rule was written as "no Node runtime hook" but the real invariant is "no secondary subprocess from inside a runtime hook". Using `.mjs` to write the sidecar file itself does not violate that invariant.
  - **Why flip now**: Maintaining two shell templates (POSIX `sh -lc '...'` and Windows `powershell -NoProfile -Command "..."`) has repeatedly produced quoting/portability bugs. The shell-mismatch helper (`contains_managed_runtime_shell_mismatch`) exists specifically to migrate files written on the wrong host — evidence that the split is not sustainable. A single Node script collapses this to one code path.
  - **Scope of the change**: Only the runtime-state hook migrates to Node. Guard hooks are already Node. The subprocess-spawn ban is preserved as a hard contract in FR-022 / FR-055 and asserted in a new integration test.
  - **Legacy detection**: Extended to match three historical shapes — `gwt-forward-hook.mjs`, direct `sh -lc '...GWT_MANAGED_HOOK=runtime-state...'`, and PowerShell equivalents — so worktrees upgraded in place migrate cleanly in one pass.
  - **Alternative considered**: Reuse `gwt-forward-hook.mjs` after stripping its subprocess-spawn code. Rejected: the legacy detection logic already pattern-matches the file name, so keeping the name would create self-matching ambiguity. Clean new name (`gwt-runtime-state.mjs`) avoids all collision with the legacy detector.
  - **Alternative considered**: Keep `sh -lc` and accept the two-template burden. Rejected: unifies with the four guard hooks (already Node), reduces test surface, removes an entire helper (`command_shell_mismatch`) and removes the platform dispatcher from the hot path.

## Technology Choices (addendum)

| Component | Choice | Rationale |
|-----------|--------|-----------|
| Runtime-state hook script | `gwt-runtime-state.mjs` (Node, write-only) | Single cross-platform authoring surface; no shell quoting; no subprocess spawn |
| Legacy detection | Multi-pattern: `gwt-forward-hook.mjs` OR `sh -lc '...GWT_MANAGED_HOOK=runtime-state...'` OR `powershell ... GWT_MANAGED_HOOK ... runtime-state ...` | Covers every historical shape so upgrade path is a single pass |
| Subprocess invariant | Integration test traces child processes during `node gwt-runtime-state.mjs` execution | Hard guarantee that no secondary `gwt`/`gwt-tauri` is spawned, enforced in CI |

## Technology Choices

| Component | Choice | Rationale |
|-----------|--------|-----------|
| File bundling | `include_dir` crate | Recursively embeds directories with structure. Simpler than custom build.rs code generation. |
| YAML validation | `serde_yaml` (build dependency) | Catches syntax errors at compile time. Not used at runtime. |
| Git exclude | `# gwt-managed-begin` / `# gwt-managed-end` markers | Same marker pattern as hooks.rs. Preserves user entries. |
| settings.local.json | Existing `hooks.rs` merge logic | Reuses proven merge/backup/recovery code. |

## Open Questions

- None remaining for Phase 2. All design decisions confirmed by user interview (2026-04-04).
