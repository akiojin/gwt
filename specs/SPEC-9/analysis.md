# Analysis: SPEC-9 - Infrastructure — build distribution, Docker UI, embedded skills, hooks merge

## Analysis Report: SPEC-9

Status: AUTO-FIXABLE (Phase 2 redesigned, new tasks pending)

## Checks

- Clarification completeness: no `[NEEDS CLARIFICATION]` markers remain in `spec.md`.
- Artifact completeness: `spec.md`, `plan.md`, `tasks.md`, supporting docs, `checklists/*`, `progress.md`, and `analysis.md` are present.
- Task traceability snapshot: Phase 1 (Docker UI): 21/21 completed. Phase 2 (Embedded Skills): 0/45 completed (redesigned 2026-04-04). Phase 3 (Hooks Merge): 31/31 completed. Phase 4 (Build Distribution): 11/11 completed.
- FR numbering: FR-001 through FR-023. No gaps or duplicates.
- SC numbering: SC-001 through SC-015. No gaps or duplicates.

## Phase 2 Redesign Summary (2026-04-04)

The Embedded Skills subsystem was completely redesigned:

### Removed

- `BuiltinSkill` enum (9 hardcoded variants) — replaced by build-time file bundling
- `SKILL_CATALOG` constant — unused dead code
- `register_builtins()` — no longer needed
- `skill_fields()` in TUI Settings — no runtime skill management
- YAML frontmatter parsing at runtime — skill interpretation is Claude Code/Codex responsibility

### Added

- **Phase 2**: Build-time bundling via `include_dir` crate (FR-009 through FR-011)
- **Phase 2b**: Runtime distribution to worktrees (FR-012 through FR-015)
- **Phase 2c**: Quality improvement per Anthropic guidelines (FR-016 through FR-018)

### Key Design Decisions

1. gwt treats skill files as opaque blobs — bundle and write, no parsing
2. Build-time YAML validation only (syntax errors caught at compile time)
3. Full overwrite on every agent launch (no diff-based sync)
4. Distribution to `.claude/`, `.codex/`, `.agents/` simultaneously
5. `.git/info/exclude` managed with `# gwt-managed-begin/end` markers
6. `settings.local.json` generated with hooks merge preserving user hooks

## Consistency Issues Found and Fixed

1. **FR numbering collision**: Hooks Merge FRs (old FR-013 through FR-017) collided with new Embedded Skills FRs. Renumbered to FR-019 through FR-023.
2. **Task ID collision**: Phase 2 new tasks (T-022 through T-066) collided with Phase 3 legacy tasks. Phase 3 renumbered to T-100 through T-130.

## Next

- Implement Phase 2 tasks (T-022 through T-066).
- Phase 2c (quality improvement) can run in parallel with Phase 2/2b.
- After Phase 2 completion, run final analysis gate before closing SPEC.
