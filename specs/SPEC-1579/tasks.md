<!-- GWT_SPEC_ARTIFACT:doc:tasks.md -->
doc:tasks.md

## Phase 1: Workflow Canonicalization

- [x] T001 Confirm #1579 as the canonical workflow/registration spec.
- [x] T002 Keep #1354 and #1643 linked with explicit ownership boundaries (storage/API and completion gate are now part of #1579).
- [x] T003 Redefine stop conditions so clear owners and auto-fixable gaps continue without user intervention.

## Phase 2: Skill and Command Execution Ownership

- [x] T004 Update `gwt-issue-register` and `gwt-issue-resolve` so they continue into the owning workflow when the correct destination is clear.
- [x] T005 Update `gwt-spec-register`, `gwt-spec-clarify`, `gwt-spec-plan`, `gwt-spec-tasks`, `gwt-spec-analyze`, and `gwt-spec-ops` for self-driving orchestration.
- [x] T006 Add `gwt-spec-implement` as the explicit implementation owner after `CLEAR` analysis.
- [x] T007 Update `gwt-pr` and `gwt-pr-fix` so routine base merges and high-confidence PR fixes proceed autonomously.

## Phase 3: Migration and Distribution

- [x] T008 Keep migration scope covering both local `specs/SPEC-*` and body-canonical issue conversion.
- [x] T009 Update registration/catalog assets and managed skill output to distribute the autonomy changes.

## Phase 4: Verification

- [x] T010 Verify markdown quality for all changed skill and command docs.
- [x] T011 Verify registration asset tests in `gwt-core`.
- [x] T012 Verify `gwt-core` clippy passes with the updated catalog.
- [x] T013 Sync #1579 artifacts to match the implemented autonomy model.

## Phase 5: GitHub Transport Policy

- [x] T014 Add REST-first / GraphQL-only-where-needed transport policy to #1579.
- [x] T015 Plan `gwt-pr` migration to REST endpoints for PR list/create/update/view.
- [x] T016 Plan `gwt-pr-check` migration to REST PR status lookups.
- [x] T017 Plan `gwt-pr-fix` migration so CI/reviews/comments use REST and review-thread operations remain GraphQL-only.

## Phase: Skill Implementation (Complete)

- [x] T018 gwt-issue-register: Issue/SPEC 登録スキル実装
- [x] T019 gwt-issue-resolve: Issue 解決ワークフロースキル実装
- [x] T020 gwt-issue-search: セマンティック Issue 検索スキル実装
- [x] T021 gwt-spec-register: SPEC コンテナ作成・初期 spec.md シードスキル実装
- [x] T022 gwt-spec-clarify: SPEC 仕様明確化スキル実装
- [x] T023 gwt-spec-ops: SPEC オーケストレーション中央ワークフロースキル実装
- [x] T024 gwt-spec-plan: プランニングアーティファクト生成スキル実装
- [x] T025 gwt-spec-tasks: タスクリスト生成スキル実装
- [x] T026 gwt-spec-analyze: 実装前分析ゲートスキル実装
- [x] T027 gwt-spec-implement: テストファースト実装オーナースキル実装
- [x] T028 gwt-spec-to-issue-migration: レガシー仕様移行スキル実装
- [x] T029 gwt-pr: REST-first PR 作成・更新スキル実装
- [x] T030 gwt-pr-check: REST-first PR 状態チェックスキル実装
- [x] T031 gwt-pr-fix: CI/review/conflict 修正スキル実装
- [x] T032 gwt-agent-dispatch: PTY ベース Agent ディスパッチスキル実装
- [x] T033 gwt-file-search: セマンティックファイル検索スキル実装

## Phase: Artifact-first Storage/API (from SPEC-1327)

- [ ] T034 Add RED tests for `doc:*` artifact parsing and mixed-mode precedence.
- [ ] T035 Add `Doc` to `SpecIssueArtifactKind`.
- [ ] T036 Reconstruct `SpecIssueDetail.sections` from `doc:*` artifacts first.
- [ ] T037 Preserve body-section fallback for legacy issues.
- [ ] T038 Extend Tauri `issue_spec` command serialization for `doc` artifacts.
- [ ] T039 Verify builtin/MCP/tooling paths can reason about `doc`, `contract`, and `checklist` together.
- [ ] T040 Extend migration scope to support body-canonical issue -> artifact-first issue conversion.
- [ ] T041 Keep local `specs/SPEC-*` migration as a supported path.
- [ ] T042 Run targeted Rust and Tauri tests for artifact-first and legacy spec bundles.

## Phase: Completion Gate (from SPEC-1730)

- [ ] T043 Define explicit post-implementation completion-gate semantics for `gwt-spec-*`.
- [ ] T044 Define rollback rules for false completion state in issue artifacts and progress comments.
- [ ] T045 Update `gwt-spec-ops` to require completion-gate reconciliation before final done state.
- [ ] T046 Update `gwt-spec-analyze` docs so `CLEAR` is scoped to pre-implementation readiness only.
- [ ] T047 Update `gwt-spec-implement` docs to require post-implementation artifact/code reconciliation.
- [ ] T048 Update `plugins/gwt/commands/gwt-spec-*.md` to match the new exit rules.
- [ ] T049 Define valid `checklist:tdd.md` structure for workflow-owned specs.
- [ ] T050 Add validation expectations for stale or malformed checklist artifacts.
- [ ] T051 Document how acceptance scenarios map back to verification tasks and completion claims.
- [ ] T052 Re-open #1654 completion state under the new gate.
- [ ] T053 Repair `#1654` task/checklist/progress consistency before declaring completion again.
- [ ] T054 Use #1654 as the first acceptance case for the new completion-gate workflow.
- [ ] T055 Verify skill docs and command docs describe the same owner and exit behavior.
- [ ] T056 Verify malformed or stale checklist artifacts are treated as blockers.
- [ ] T057 Verify #1654 cannot return to done state until artifacts and implementation agree.
