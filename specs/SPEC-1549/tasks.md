### Phase S: Setup
- [ ] T001 [S] [US-1] Rust 側の既存 Project Mode 実装の分析・状態遷移図作成
- [ ] T002 [S] [US-1] `ILeadOrchestrationService` インターフェース設計（モード切替なし・常時アクティブ）
- [ ] T003 [S] [US-1] VContainer DI 登録

### Phase F: Foundation
- [ ] T004 [F] [US-1] `LeadAgent` の C# 実装（API 直接呼び出し、完全自律動作）
- [ ] T005 [F] [US-1] Lead が Agent ツールとしてコンテキスト（コードベース、SPEC、Git、Issue）を参照する仕組み
- [ ] T006 [F] [US-7] Lead キャラクター性設定システム（名前、口調、ペルソナリティ）
- [ ] T007 [F] [US-12] Agent エラー対処状況の監視・進捗反映（**介入なし、監視のみ**）
- [ ] T008 [F] [US-2] `AgentWorker` の C# 実装（worktree 作業実行）
- [ ] T009 [F] [US-5] セッション永続化・復元の実装（SPEC 生成状態含む）

### Phase U: User Story
- [ ] T010 [U] [US-7] Lead 雇用システムの実装（候補一覧、選択 UI（アバター+名前+性格タイプ+口調サンプル付きプレビュー）、雇用フロー）
- [ ] T011 [U] [US-8] Lead 解雇・再雇用システムの実装（解雇フロー、候補再選択）
- [ ] T012 [U] [US-8] 引継ぎドキュメント生成の実装（全コンテキスト含む: SPEC 状態、タスク進捗、エージェント状況、進行中の計画）
- [ ] T013 [U] [US-10] Lead 候補定義（見た目・声・性格バリエーション、AI 性能は共通、LimeZu アセットから選択）
- [ ] T014 [U] [US-9] Issue close 判断の実装（Lead AI による状況判断、worktree 削除提案、ユーザー承認フロー）
- [ ] T015 [U] [US-2] Lead Git 権限スコープの実装（worktree 作成→push→PR作成→merge→削除、force push / rebase 禁止）
- [ ] T016 [U] [US-13] Lead 専用 SPEC 内蔵ツール実装（LLM function calling 用に専用設計）
- [ ] T017 [U] [US-13] SPEC 生成インタビューフロー実装（Lead がユーザーに質問 → 回答を構造化）
- [ ] T018 [U] [US-13] SPEC テンプレートシステム実装（Spec/Plan/Tasks/TDD/Research/DataModel/Quickstart/Contracts/Checklists）
- [ ] T019 [U] [US-14] SPEC → GitHub Issue 自動作成/更新機能実装（`gwt-spec` ラベル、親子関係設定）
- [ ] T020 [U] [US-15] 既存 SPEC 整合性チェック実装（重複・矛盾検出）
- [ ] T021 [U] [US-14] SPEC ライフサイクル管理実装（Draft → Review → Approved → In Progress → Completed）
- [ ] T022 [U] [US-13] TDD セクションのテストコード自動生成（NUnit / vitest）
- [ ] T023 [U] [US-16] 実装中の知見に基づく SPEC 自律更新提案
- [ ] T024 [U] [US-3] スタジオ連携（キャラクター生成・状態同期）

### Phase FIN: Finalization
- [ ] T025 [FIN] [US-1] ユニットテスト作成（SPEC 生成テンプレート、整合性チェック含む）
- [ ] T026 [FIN] [US-1] 統合テスト作成
- [ ] T027 [FIN] [US-1] NFR-001〜004 パフォーマンス検証
