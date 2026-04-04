# Progress: SPEC-9 - Infrastructure

## Progress

- Status: `in-progress`
- Phase: `Implementation`
- Task progress: Phase 1: 21/21, Phase 2: 0/45 (redesigned), Phase 3: 31/31, Phase 4: 11/11
- Artifact refresh: `2026-04-04T18:00:00Z`

## Done

- Phase 1 (Docker UI): All screens implemented and tested.
- Phase 3 (Hooks Merge): All merge logic, backup/recovery, and polish completed.
- Phase 4 (Build Distribution): Release workflow and npm distribution verified.
- Supporting artifacts cover the full infrastructure umbrella.

## Phase 2 Redesign (2026-04-04)

Embedded Skills subsystem was completely redesigned based on Anthropic/OpenAI skill authoring guidelines:

- **Removed**: `BuiltinSkill` enum, `SKILL_CATALOG`, `register_builtins()`, TUI skill toggle
- **Added**: Build-time bundling (`include_dir`), runtime distribution to worktrees, `.git/info/exclude` management, `settings.local.json` generation
- **Quality**: All 21 SKILL.md files to be rewritten per Anthropic guidelines (third-person descriptions, Progressive Disclosure, `allowed-tools` frontmatter)
- **YAML validation**: Build-time only (syntax check). No runtime parsing.

## Next

- Implement Phase 2 (Build-Time Bundling): T-022 through T-034
- Implement Phase 2b (Runtime Distribution): T-035 through T-055
- Implement Phase 2c (Quality Improvement): T-056 through T-066 (parallel)
- After Phase 2 completion, run final analysis gate
