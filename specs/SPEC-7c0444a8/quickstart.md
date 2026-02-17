# Quickstart: GUI Worktree Summary 7タブ再編（Issue #1097）

## 前提

1. `gwt-gui` 依存をインストール済み (`cd gwt-gui && pnpm install`)
2. `gh` が利用可能（Issue/PR/Workflow 取得検証用）

## 手動確認手順

1. `cargo tauri dev` を起動し、対象プロジェクトを開く。
2. Worktree を選択して Summary パネルを表示する。
3. タブ列が `Quick Start / Summary / Git / Issue / PR / Workflow / Docker` で固定表示されることを確認する。
4. `feature/issue-<number>` ブランチで Issue タブを開き、該当 Issue のみ表示されることを確認する。
5. PR があるブランチで PR/Workflow タブを確認し、PRなしブランチでは空状態になることを確認する。
6. Docker タブで current context と Quick Start 履歴が併記されることを確認する。

## 自動テスト

1. `cd gwt-gui && pnpm test src/lib/components/WorktreeSummaryPanel.test.ts`
2. 必要に応じて backend command テストを実行:
   - `cargo test -p gwt-tauri commands::issue`
   - `cargo test -p gwt-tauri commands::pullrequest`
