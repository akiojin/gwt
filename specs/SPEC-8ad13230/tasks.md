# タスクリスト: SPEC-8ad13230 Agent Mode Issue-first Spec Bundle CRUD

## Phase 1: セットアップ

- [x] T001 [P] [共通] 仕様と実装対象の差分を確定する `specs/SPEC-8ad13230/spec.md`
- [x] T002 [P] [共通] 実装計画を作成する `specs/SPEC-8ad13230/plan.md`

## Phase 2: 基盤

- [x] T003 [US1] `SpecIssueSections` に `tdd/contracts/checklists` を追加する `crates/gwt-core/src/git/issue_spec.rs`
- [x] T004 [US2] artifact コメントのモデルと parser を追加する `crates/gwt-core/src/git/issue_spec.rs`
- [x] T005 [US2] artifact コメントの `upsert/list/delete` を実装する `crates/gwt-core/src/git/issue_spec.rs`
- [x] T006 [US2] export を更新する `crates/gwt-core/src/git.rs`

## Phase 3: ストーリー 1（Issue bundle + TDD）

- [x] T007 [US1] Master Agent の Spec 準備で既存 section を保持する実装を追加する `crates/gwt-tauri/src/agent_master.rs`
- [x] T008 [US1] Master Agent の system prompt を成果物完全セット前提に更新する `crates/gwt-tauri/src/agent_master.rs`
- [x] T009 [US1] `IssueSpecPanel` に TDD/Contracts/Checklists を表示する `gwt-gui/src/lib/components/IssueSpecPanel.svelte`

## Phase 4: ストーリー 2（CRUD）

- [x] T010 [US2] Tauri command に artifact CRUD を追加する `crates/gwt-tauri/src/commands/issue_spec.rs`
- [x] T011 [US2] invoke ハンドラ登録を追加する `crates/gwt-tauri/src/app.rs`
- [x] T012 [US2] 内蔵ツール定義に artifact CRUD を追加する `crates/gwt-tauri/src/agent_tools.rs`
- [x] T013 [US2] 互換 API (`append_spec_contract_comment`) を upsert 動作に統合する `crates/gwt-tauri/src/commands/issue_spec.rs`

## Phase 5: ストーリー 3（MCP）

- [x] T014 [US3] MCP ツール定義を `spec_issue_artifact_upsert/list/delete` に拡張する `scripts/gwt_issue_spec_mcp.py`
- [x] T015 [US3] MCP 側の comment marker parser と CRUD を実装する `scripts/gwt_issue_spec_mcp.py`

## Phase 6: テスト（TDD）

- [x] T016 [US1] section/parser のユニットテストを追加する `crates/gwt-core/src/git/issue_spec.rs`
- [x] T017 [US1] Master Agent の merge ロジックテストを追加する `crates/gwt-tauri/src/agent_master.rs`
- [x] T018 [US2] command parse テストを追加する `crates/gwt-tauri/src/commands/issue_spec.rs`
- [x] T019 [US2] built-in tool 定義テストを更新する `crates/gwt-tauri/src/agent_tools.rs`

## Phase 7: 仕上げ・横断

- [x] T020 [P] [共通] `research.md/data-model.md/quickstart.md/contracts/tdd.md` を作成する `specs/SPEC-8ad13230/`
- [x] T021 [P] [共通] 実行結果を記録する `specs/SPEC-8ad13230/tdd.md`
