<!-- GWT_SPEC_ARTIFACT:doc:tasks.md -->
doc:tasks.md

## Phase 1: Workflow Canonicalization

- [x] T001 Confirm #1579 as the canonical workflow/registration spec.
- [x] T002 Keep #1327, #1354, and #1643 linked with explicit ownership boundaries.
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
