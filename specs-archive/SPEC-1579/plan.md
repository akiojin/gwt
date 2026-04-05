## Summary

Unified plan for the gwt-spec system: embedded-skill autonomy, local file-based storage/API, completion gate, and GitHub transport policy. Keep `gwt-spec-ops` as the workflow owner, `gwt-spec-implement` as the execution owner, shift storage from Issue-based to local `specs/SPEC-{N}/` directories, and add a post-implementation completion gate alongside the existing pre-implementation `CLEAR` gate.

## Technical Context

- Skill docs under `.claude/skills/*`
- Command docs under `.claude/commands/*`
- Registration catalog and managed-skill block generation in `crates/gwt-core/src/config/skill_registration.rs`
- Managed block sync in `CLAUDE.md`
- Existing migration support in `.claude/skills/gwt-spec-to-issue-migration`
- GitHub-backed skill surfaces: `.claude/skills/gwt-pr`, `.claude/skills/gwt-pr-check`, `.claude/skills/gwt-pr-fix`
- Core storage module: `crates/gwt-core/src/git/issue_spec.rs`
- Tauri bridge: `crates/gwt-tauri/src/commands/issue_spec.rs`
- Agent/builtin integration: `crates/gwt-tauri/src/agent_tools.rs`

## Constitution Check

- Spec before implementation: workflow ownership, stop conditions, and GitHub transport policy are fixed in the canonical spec before changing downstream skill behavior.
- Test-first: execution ownership still routes implementation through `gwt-spec-implement`, while transport migration follow-up tasks explicitly call out script and integration verification.
- No workaround-first: use an explicit REST-first / GraphQL-only-where-needed policy instead of ad hoc fallback rules spread across skills.
- Minimal complexity: keep workflow ownership in `gwt-spec-ops`, execution ownership in `gwt-spec-implement`, use one canonical reconstruction path with explicit fallback for storage, add one explicit completion gate instead of scattering ad hoc reminders, and limit GraphQL to review-thread operations that still require it.

## Project Structure

- Workflow/Storage/API/Completion canonical: #1579
- Viewer canonical: `SPEC-1776` for local SPEC viewing, `#1354` for GitHub Issue detail / legacy issue-body compatibility
- Search canonical: #1643
- Skill docs: `.claude/skills/*`
- Command docs: `.claude/commands/*`
- Registration/catalog: `crates/gwt-core/src/config/skill_registration.rs`
- Storage core: `crates/gwt-core/src/git/issue_spec.rs`
- Reference remediation case: #1654

## Complexity Tracking

- Added complexity: one explicit GitHub transport policy across embedded skills.
- Mitigation: this removes hidden GraphQL assumptions from `gwt-pr` / `gwt-pr-fix` / `gwt-pr-check` and reduces avoidable auth/quota failures.
- Added complexity: GraphQL remains for unresolved review threads and thread reply/resolve.
- Mitigation: the boundary is explicit and intentionally narrow.
- Added complexity: `Doc` artifact family and mixed-mode fallback rules.
- Mitigation: keep external command/data shapes stable (`SpecIssueDetail.sections` unchanged).
- Added complexity: one explicit exit-gate concept after implementation.
- Mitigation: removes ambiguity around when `tasks.md` may be marked complete.

## Phased Implementation

1. Keep workflow specs and skill/command docs aligned on clear-owner, minimum-stop behavior.
2. Add GitHub transport guidance: REST-first for metadata/mutations where practical, GraphQL only where unavoidable.
3. Follow up by updating `gwt-pr`, `gwt-pr-check`, and REST-eligible parts of `gwt-pr-fix` to match that policy.
4. Validate markdown plus script-level behavior for REST auth, PR lookup/create/update, CI/check reads, and GraphQL-only thread operations.
5. Add RED tests for `doc:*` artifact parsing and mixed-mode precedence. Extend artifact kind handling and detail reconstruction. Extend Tauri command serialization for `doc:*` artifacts. Define migration path for old body bundles and old local specs.
6. Specify completion-gate semantics and artifact invariants. Update `gwt-spec-ops`, `gwt-spec-analyze`, `gwt-spec-implement`, and command docs. Repair malformed checklist expectations. Apply the new rules to #1654 as the first remediation case.
