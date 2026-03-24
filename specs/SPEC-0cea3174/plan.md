## Summary

Strengthen embedded-skill autonomy so normal workflow stages do not stop on clear next actions. Keep `gwt-spec-ops` as the workflow owner, `gwt-spec-implement` as the execution owner, and add a GitHub transport policy that makes PR skills REST-first while reserving GraphQL for review-thread-specific operations.

## Technical Context

- Skill docs under `plugins/gwt/skills/*`
- Command docs under `plugins/gwt/commands/*`
- Registration catalog and managed-skill block generation in `crates/gwt-core/src/config/skill_registration.rs`
- Managed block sync in `CLAUDE.md`
- Existing migration support in `plugins/gwt/skills/gwt-spec-to-issue-migration`
- GitHub-backed skill surfaces: `plugins/gwt/skills/gwt-pr`, `plugins/gwt/skills/gwt-pr-check`, `plugins/gwt/skills/gwt-pr-fix`

## Constitution Check

- Spec before implementation: workflow ownership, stop conditions, and GitHub transport policy are fixed in the canonical spec before changing downstream skill behavior.
- Test-first: execution ownership still routes implementation through `gwt-spec-implement`, while transport migration follow-up tasks explicitly call out script and integration verification.
- No workaround-first: use an explicit REST-first / GraphQL-only-where-needed policy instead of ad hoc fallback rules spread across skills.
- Minimal complexity: keep workflow ownership in `gwt-spec-ops`, execution ownership in `gwt-spec-implement`, leave storage/viewer/search ownership in their existing specs, and limit GraphQL to review-thread operations that still require it.

## Project Structure

- Workflow canonical: #1579
- Storage/API canonical: #1327
- Viewer canonical: #1354
- Search canonical: #1643
- Skill docs: `plugins/gwt/skills/*`
- Command docs: `plugins/gwt/commands/*`
- Registration/catalog: `crates/gwt-core/src/config/skill_registration.rs`

## Complexity Tracking

- Added complexity: one explicit GitHub transport policy across embedded skills.
- Mitigation: this removes hidden GraphQL assumptions from `gwt-pr` / `gwt-pr-fix` / `gwt-pr-check` and reduces avoidable auth/quota failures.
- Added complexity: GraphQL remains for unresolved review threads and thread reply/resolve.
- Mitigation: the boundary is explicit and intentionally narrow.

## Phased Implementation

1. Keep workflow specs and skill/command docs aligned on clear-owner, minimum-stop behavior.
2. Add GitHub transport guidance to #1579: REST-first for metadata/mutations where practical, GraphQL only where unavoidable.
3. Follow up by updating `gwt-pr`, `gwt-pr-check`, and REST-eligible parts of `gwt-pr-fix` to match that policy.
4. Validate markdown plus script-level behavior for REST auth, PR lookup/create/update, CI/check reads, and GraphQL-only thread operations.
