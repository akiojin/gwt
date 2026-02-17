# タスクリスト: GUI Worktree Summary 6タブ + Quick Launchヘッダー再編（Issue #1097）

## 依存関係

- US1 完了後に US2/US3 の UI 回帰確認を行う。
- US2 の backend 選定ロジック実装（Issue/PR）は、US2 の frontend 表示実装より先に完了させる。
- 最終検証は全ストーリー完了後に実施する。

## Phase 1: セットアップ

- [ ] T001 [P] [共通] 既存 WorktreeSummaryPanel のタブ構成とテスト前提を棚卸しし、変更対象を確定する `gwt-gui/src/lib/components/WorktreeSummaryPanel.svelte` `gwt-gui/src/lib/components/WorktreeSummaryPanel.test.ts`

## Phase 2: 基盤

- [ ] T002 [US1] 6タブ固定順とヘッダー Quick Launch ボタンの UI 状態定義（タブID/選択状態/空状態ハンドリング）を追加する `gwt-gui/src/lib/components/WorktreeSummaryPanel.svelte`
- [ ] T003 [US2] ブランチ関連 Issue/PR/Workflow の取得責務を分離するための型定義を整理する `gwt-gui/src/lib/types.ts` `crates/gwt-tauri/src/commands/pullrequest.rs` `crates/gwt-tauri/src/commands/issue.rs`

## Phase 3: ストーリー 1（固定6タブ構成 + ヘッダー導線）

- [ ] T004 [US1] 6タブ固定順・ヘッダー Continue/New・タブ単位エラー分離の RED テストを追加する `gwt-gui/src/lib/components/WorktreeSummaryPanel.test.ts`
- [ ] T005 [US1] WorktreeSummaryPanel を 6タブ固定構成へ再編し、Quick Start タブを廃止してヘッダー Continue/New 導線へ移す `gwt-gui/src/lib/components/WorktreeSummaryPanel.svelte`

## Phase 4: ストーリー 2（Issue/PR/Workflow のブランチ文脈化）

- [ ] T006 [US2] Issue タブの「branch issue のみ表示 / 非該当は空状態」RED テストを追加する `gwt-gui/src/lib/components/WorktreeSummaryPanel.test.ts`
- [ ] T007 [US2] PR/Workflow の「open優先PR選定 / PRなし空状態」RED テストを追加する `gwt-gui/src/lib/components/WorktreeSummaryPanel.test.ts`
- [ ] T008 [US2] ブランチ名 `issue-<number>` 連携のみで Issue を返すロジックを実装し、fallback 一覧表示を排除する `crates/gwt-tauri/src/commands/issue.rs` `gwt-gui/src/lib/components/WorktreeSummaryPanel.svelte`
- [ ] T009 [US2] PR 選定と Workflow 表示を branch PR 前提で整合させる実装を追加する `crates/gwt-tauri/src/commands/pullrequest.rs` `gwt-gui/src/lib/components/WorktreeSummaryPanel.svelte`

## Phase 5: ストーリー 3（Launch導線/Summary/Docker の責務分離）

- [ ] T010 [US3] Summary から Quick Start が除外されること、ヘッダー Continue/New の空状態無効化、Docker 併記表示の RED テストを追加する `gwt-gui/src/lib/components/WorktreeSummaryPanel.test.ts`
- [ ] T011 [US3] Launch導線・Summary・Docker の表示責務を分離し、Docker 現在状態と履歴を併記する `gwt-gui/src/lib/components/WorktreeSummaryPanel.svelte`

## Phase 6: 仕上げ・横断

- [ ] T012 [P] [共通] フロントエンドの対象テストを実行して回帰を確認する `gwt-gui/src/lib/components/WorktreeSummaryPanel.test.ts`
- [ ] T013 [P] [共通] 必要な backend command テストを実行して Issue/PR/Workflow 選定ロジックを確認する `crates/gwt-tauri/src/commands/issue.rs` `crates/gwt-tauri/src/commands/pullrequest.rs`
- [ ] T014 [共通] 受け入れ条件（6タブ固定順・ヘッダーContinue/New・Summary分離・Issue限定・PR/Workflow/Docker空状態）の実機スモークを記録する `specs/SPEC-7c0444a8/quickstart.md`

## 並列実行候補

- T004 と T006/T007/T010 は同一ファイル編集の衝突を避けるため直列推奨。
- T008（Issue）と T009（PR/Workflow）は backend 側を並列で進められる。
- T012 と T013 は並列実行可能。

## MVP

- MVP は US1 + US2（T004〜T009）とし、6タブ固定構成 + ヘッダー導線と branch 文脈の Issue/PR/Workflow を先に成立させる。
