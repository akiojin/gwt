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

## Technology Choices

| Component | Choice | Rationale |
|-----------|--------|-----------|
| File bundling | `include_dir` crate | Recursively embeds directories with structure. Simpler than custom build.rs code generation. |
| YAML validation | `serde_yaml` (build dependency) | Catches syntax errors at compile time. Not used at runtime. |
| Git exclude | `# gwt-managed-begin` / `# gwt-managed-end` markers | Same marker pattern as hooks.rs. Preserves user entries. |
| settings.local.json | Existing `hooks.rs` merge logic | Reuses proven merge/backup/recovery code. |

## Open Questions

- None remaining for Phase 2. All design decisions confirmed by user interview (2026-04-04).
