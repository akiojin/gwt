# Progress: SPEC-9 - Infrastructure

## Progress

- Status: `in-progress`
- Phase: `Implementation`
- Task progress: Phase 1: 21/21, Phase 2: 13/13, Phase 2b: 41/41, Phase 2c: 11/11, Phase 3: 31/31, Phase 4: 11/11, Phase 5: 14/20, Phase 6: 11/11
- Artifact refresh: `2026-04-07T07:45:00Z`

## Done

- Phase 1 (Docker UI): All screens implemented and tested.
- Phase 2 (Build-Time Bundling): `BuiltinSkill` enum removed, `include_dir` bundling, build.rs YAML validation, validate module with tests.
- Phase 2b (Runtime Distribution): `distribute_to_worktree()`, `update_git_exclude()`, `generate_settings_local()`, and `generate_codex_hooks()` are implemented and tested. Agent launch integration is wired, tracked `.codex/hooks.json` files are preserved by default but tracked legacy gwt runtime forward-hook files are migrated in place, and Claude/Codex live-state hooks now write `GWT_SESSION_RUNTIME_PATH` without a Node runtime forwarder.
- Codex launch configs now explicitly enable `codex_hooks`, matching the current OpenAI Codex hooks contract so gwt-managed Codex sessions actually execute available `hooks.json` files.
- Codex launch configs now also add the current runtime PID namespace directory as an explicit writable root, so runtime hooks can still write `~/.gwt/sessions/runtime/<pid>/...` while Codex is sandboxed with `workspace-write`.
- Because the persisted session id is only known during launch materialization, `app.rs` now appends the effective Codex runtime writable root after the session record is created, keeping the final spawned argv aligned with the injected `GWT_SESSION_RUNTIME_PATH`.
- SPEC-9 now also records the current Codex product caveat: interactive sessions may not emit `SessionStart` before the first prompt, so downstream launch code is allowed to bootstrap a `Running` runtime sidecar until the first real hook event arrives.
- Phase 2c (Quality Improvement): All 21 SKILL.md rewritten per Anthropic guidelines. Progressive Disclosure applied to 7 complex skills. All files under 200 lines.
- Phase 3 (Hooks Merge): All merge logic, backup/recovery, and polish completed.
- Phase 4 (Build Distribution): Release workflow and npm distribution verified.
- Phase 5 (Skill Consolidation): the consolidated 8-skill structure is implemented; only the explicit standalone verification tasks remain open.
- Phase 6 (Search Runtime Contract Recovery): shared search runtime bootstrap, canonical `index-files` / `search-files` naming, and `index-issues --project-root` documentation are now aligned.
- Search runtime bootstrap now validates Python candidates before venv creation and surfaces install guidance when Python 3.9+ is unavailable.
- Phase 6 keeps the runtime contract on `search-files` / `index-files`, but the standalone user-facing skill / slash-command surface is restored to `gwt-project-search` because the workflow semantically finds related implementation areas inside the project.
- Bundled Claude/Codex skills now distribute canonical `gwt-project-search` assets, while `gwt-file-search` assets are intentionally absent so public naming stays aligned with project-search semantics.
- Phase 6 follow-up now targets file-index quality itself: implementation search must ignore embedded skill/spec/snapshot noise and split docs into a separate collection so `search-files` behaves like code discovery instead of generic markdown retrieval.
- The repo-tracked runner now writes separate code/docs collections, exposes `search-files-docs` for project docs, and keeps `search-files` focused on implementation files after excluding `.claude/`, `.codex/`, `specs/`, `specs-archive/`, `tasks/`, and `*.snap`.

## Next

- Run completion gate: reconcile spec.md, tasks.md, analysis.md, checklists
- Update analysis.md to CLEAR
- Verify the remaining Phase 5 standalone skill checks
- Consider PR creation
