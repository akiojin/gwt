## Summary

Shift the spec storage contract from body-canonical bundles to artifact-first comments while preserving legacy compatibility and shared CRUD APIs.

## Technical Context

- Core module: `crates/gwt-core/src/git/issue_spec.rs`
- Tauri bridge: `crates/gwt-tauri/src/commands/issue_spec.rs`
- Agent/builtin integration: `crates/gwt-tauri/src/agent_tools.rs`
- Migration helper scope: `gwt-spec-to-issue-migration`

## Constitution Check

- Spec before implementation: yes, storage contract is fixed here before code changes.
- Test-first: parser and reconstruction tests must be added before refactoring logic.
- No workaround-first: use one canonical reconstruction path with explicit fallback, not duplicated reader branches.
- Minimal complexity: preserve `SpecIssueDetail.sections` shape and absorb change inside the backend.

## Project Structure

- `gwt-core`: artifact parsing, detail reconstruction, legacy fallback
- `gwt-tauri`: command exposure and serialization
- migration: legacy local specs and old issue body bundles

## Complexity Tracking

- New complexity: `Doc` artifact family and mixed-mode fallback rules
- Mitigation: keep external command/data shapes stable

## Phased Implementation

1. Add RED tests for `doc:*` artifact parsing and mixed-mode precedence.
2. Extend artifact kind handling and detail reconstruction.
3. Extend Tauri command serialization and tooling hooks for `doc:*` artifacts.
4. Define migration path for old body bundles and old local specs.
