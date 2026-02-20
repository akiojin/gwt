# タスクリスト: URLクリック時の外部ブラウザ起動統一

## 依存関係

- US1 を先に完了し、US2 は US1 の共通実装を前提に進める。
- Tauri 側の plugin/permission 設定はフロント実装と同一PRで反映する。

## Phase 1: セットアップ

- [x] T001 [US1] 仕様反映（spec/plan/tasks）を更新して実装対象を確定する `specs/SPEC-d95a7e0c/spec.md`

## Phase 2: 基盤

- [x] T002 [US1] 共通 URL オープナーを追加する `gwt-gui/src/lib/openExternalUrl.ts`
- [x] T003 [US1] 共通 URL オープナーのユニットテストを追加する（TDD: RED→GREEN） `gwt-gui/src/lib/openExternalUrl.test.ts`

## Phase 3: ストーリー 1（全画面で外部ブラウザ起動）

- [x] T004 [US1] App 全体のリンククリック委譲を追加して `http/https` を共通オープナーへ集約する `gwt-gui/src/App.svelte`
- [x] T005 [US1] Terminal の URL クリック処理を共通オープナーに統一する `gwt-gui/src/lib/terminal/TerminalView.svelte`
- [x] T006 [US1] 既存の個別 URL オープン処理を共通オープナーへ寄せる `gwt-gui/src/lib/components/IssueListPanel.svelte`
- [x] T007 [US1] Workflow URL オープン処理を共通オープナーへ寄せる `gwt-gui/src/lib/components/WorktreeSummaryPanel.svelte`
- [x] T008 [US1] Terminal 側のリンククリックテストを更新する `gwt-gui/src/lib/terminal/TerminalView.test.ts`
- [x] T009 [US1] Workflow URL オープンテストを更新する `gwt-gui/src/lib/components/WorktreeSummaryPanel.test.ts`

## Phase 4: ストーリー 2（安全スキーム制限）

- [x] T010 [US2] 非許可スキーム拒否のテストを追加・更新する `gwt-gui/src/lib/openExternalUrl.test.ts`
- [x] T011 [US2] 許可スキーム判定を `http/https` のみに固定する `gwt-gui/src/lib/openExternalUrl.ts`

## Phase 5: 仕上げ・横断

- [x] T012 [共通] Tauri shell plugin の依存と初期化を追加する `crates/gwt-tauri/Cargo.toml`
- [x] T013 [共通] Tauri capability に `shell:allow-open` を追加する `crates/gwt-tauri/capabilities/default.json`
- [x] T014 [共通] 対象テストを実行して回帰確認する `gwt-gui/src/lib/openExternalUrl.test.ts`
