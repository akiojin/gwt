# タスクリスト: エージェントモードUI改善

## Phase 1: セットアップ

- [ ] T001 [P] [US1] [準備タスク] 既存Agent Mode UI要件と仕様差分の確認 specs/SPEC-ba3f610c/spec.md

## Phase 2: 基盤

- [ ] T002 [US1] [基盤実装] AgentModePanelの入力/送信状態の現状整理 gwt-gui/src/lib/components/AgentModePanel.svelte

## Phase 3: ストーリー 1

- [ ] T003 [US1] [テスト] IME送信抑止/送信中スピナー/自動スクロールのテスト追加 gwt-gui/src/lib/components/AgentModePanel.test.ts
- [ ] T004 [US1] [実装] IME変換中のEnter送信抑止を追加 gwt-gui/src/lib/components/AgentModePanel.svelte
- [ ] T005 [US1] [実装] チャットバブル表示/送信中スピナー/自動スクロールを追加 gwt-gui/src/lib/components/AgentModePanel.svelte

## Phase 4: 仕上げ・横断

- [ ] T006 [P] [共通] 仕様・計画・タスクの更新 specs/SPEC-ba3f610c/spec.md

## Phase 5: Session Summaryタブ回帰修正（2026-02-13）

- [x] T007 [US1] [テスト] ブランチモードのタブ固定仕様テストを追加 gwt-gui/src/lib/components/MainArea.test.ts
- [x] T008 [US1] [テスト] 既定タブ/復元条件のユニットテストを追加 gwt-gui/src/lib/appTabs.test.ts
- [x] T009 [US1] [実装] Appの初期タブ/再初期化/復元ロジックからSession Summary依存を削除 gwt-gui/src/App.svelte
- [x] T010 [US1] [実装] Tab型からsummaryを除去して再発を防止 gwt-gui/src/lib/types.ts
