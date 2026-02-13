# タスクリスト: エージェントモード（MA + Agentビュー）

## Phase 1: セットアップ

- [x] T001 [P] [共通] `SPEC-ba3f610c` のFR-030〜FR-036/TR-003差分確認と対象ファイル特定 `specs/SPEC-ba3f610c/spec.md`

## Phase 2: 基盤

- [x] T002 [US1] [基盤] Agent Mode状態モデルに Task/SubAgent/worktree表示情報を追加 `crates/gwt-tauri/src/commands/sessions.rs`
- [x] T003 [US1] [基盤] フロント型定義に Agentビュー用モデルを追加 `gwt-gui/src/lib/types.ts`

## Phase 3: ユーザーストーリー 1（MAチャット）

- [x] T004 [US1] [テスト] IME送信抑止/送信中スピナー/自動スクロールの既存テストを維持・拡張 `gwt-gui/src/lib/components/AgentModePanel.test.ts`
- [x] T005 [US1] [実装] MAチャット表示と既存送信UXを回帰なく維持 `gwt-gui/src/lib/components/AgentModePanel.svelte`

## Phase 4: ユーザーストーリー 2（タスク一覧 + 選択連動）

- [x] T006 [US2] [テスト] タスク一覧の表示順（`running > pending > failed > completed`）テストを追加 `gwt-gui/src/lib/components/AgentSidebar.test.ts`
- [x] T007 [US2] [実装] Agentビューに状態付きタスク一覧を実装 `gwt-gui/src/lib/components/AgentSidebar.svelte`
- [x] T008 [US2] [テスト] タスク選択時に下部へ担当サブエージェント一覧が表示されるテストを追加 `gwt-gui/src/lib/components/AgentSidebar.test.ts`
- [x] T009 [US2] [実装] タスク選択と下部サブエージェント一覧連動を実装（全件表示） `gwt-gui/src/lib/components/AgentSidebar.svelte`

## Phase 5: ユーザーストーリー 3（割当/再計画同期 + worktree表示）

- [x] T010 [US3] [テスト] 1タスク複数担当の表示と再計画時の現在担当のみ表示テストを追加 `gwt-gui/src/lib/components/AgentSidebar.test.ts`
- [x] T011 [US3] [実装] 再割当時の現在担当のみ表示ロジックを実装 `crates/gwt-tauri/src/commands/sessions.rs`
- [x] T012 [US3] [テスト] worktree相対表示 + 詳細/ホバーで絶対パス表示テストを追加 `gwt-gui/src/lib/components/AgentSidebar.test.ts`
- [x] T013 [US3] [実装] worktree表示仕様（相対デフォルト、絶対詳細）を実装 `gwt-gui/src/lib/components/AgentSidebar.svelte`

## Phase 6: 仕上げ・横断

- [x] T014 [P] [共通] バックエンド状態変換テストを追加（再計画同期/現在担当のみ） `crates/gwt-tauri/src/commands/sessions.rs`
- [x] T015 [P] [共通] MAのSpec Kit成果物4点チェック（`spec.md`/`plan.md`/`tasks.md`/`tdd.md`）失敗テストを追加 `crates/gwt-tauri/src/agent_master.rs`
- [x] T016 [P] [共通] MAのSpec Kit成果物4点チェック実装（不足時は実行ブロック） `crates/gwt-tauri/src/agent_master.rs`
- [x] T017 [P] [共通] `tdd.md`生成フローと保存処理を追加 `crates/gwt-tauri/src/agent_master.rs`
- [x] T018 [P] [共通] 仕様・計画・タスクの最終同期確認（`tdd.md`含む） `specs/SPEC-ba3f610c/spec.md`
