# 調査メモ: GUI Worktree Summary 7タブ再編（Issue #1097）

## 1. 現状の問題

- `WorktreeSummaryPanel` は複数情報を同一ビューに混在表示しており、Quick Start と Summary の責務境界が曖昧。
- Issue 表示に branch-related 以外の fallback が入ると、現在作業中の Issue 追跡を妨げる。

## 2. 既存資産の再利用

- Quick Start 履歴取得: `get_branch_tool_sessions` 系コマンド
- AI Summary 表示: 既存 markdown 表示ロジック（Summary タブ相当）
- Git 表示: 既存 `GitSection`
- PR/Workflow: 既存 PR status / check suite 取得ロジック
- Docker 状態: `detect_docker_context`

## 3. 設計上の判断

- タブは固定7つを常時表示し、データ有無でタブを消さない。
- 取得失敗はタブ単位に閉じ込める（global error にしない）。
- Issue は `issue-<number>` パターン一致時のみ取得対象とし、open issues 一覧には戻さない。

## 4. リスクと緩和

- リスク: 既存テストが旧レイアウト前提で多数失敗する可能性
  - 緩和: 7タブ固定順・空状態・主要成功ケースでテスト観点を再定義する
- リスク: PR/Workflow 取得失敗時の UX 低下
  - 緩和: 原因が分かる短い空状態/エラーメッセージを表示する
