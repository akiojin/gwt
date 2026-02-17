# TDDノート: Issue タブ — GitHub Issue 一覧・詳細・フルフロー

## 対象

- `crates/gwt-tauri/src/commands/issue.rs` — バックエンド Issue コマンド拡張
- `gwt-gui/src/lib/components/IssueListPanel.svelte` — Issue 一覧・詳細 UI
- `gwt-gui/src/lib/components/MarkdownRenderer.svelte` — GFM Markdown レンダリング
- `gwt-gui/src/lib/components/AgentLaunchForm.svelte` — Issue プリフィル拡張
- `gwt-gui/src/lib/types.ts` — TypeScript 型定義

## テスト戦略

1. バックエンド: Rust ユニットテストで型拡張・コマンド追加の正常系/エラー系を検証
2. フロントエンド: vitest + @testing-library/svelte でコンポーネント描画・インタラクション検証
3. GFM Markdown レンダリングは XSS 安全性を重点テスト
4. 既存テストの回帰確認を最優先（IssueSpecPanel・AgentLaunchForm の既存テスト）
5. テストファースト: 各フェーズで実装コードより先にテストを書く

## Red / Green 記録

### T007: バックエンド拡張テスト

- **Red**: GitHubIssueInfo に body/assignees/comments_count 等が存在しない → テスト失敗
- **Green**: 型拡張 + gh CLI の JSON 出力パース実装後、テスト通過

### T009-T010: MarkdownRenderer テスト

- **Red**: MarkdownRenderer コンポーネントが存在しない → import エラー
- **Green**: marked + DOMPurify による GFM レンダリング実装後
  - 見出し (h1-h6) のレンダリング
  - コードブロック（言語指定あり/なし）
  - テーブル（GFM 拡張）
  - チェックボックスリスト（GFM 拡張）
  - 取り消し線（GFM 拡張）
  - リンク（target="_blank" 付き）
  - 画像
  - XSS 攻撃文字列のサニタイズ（`<script>`, `onerror`, `javascript:` 等）

### T011-T016: IssueListPanel 一覧テスト

- **Red**: IssueListPanel コンポーネントが存在しない → import エラー
- **Green**: 以下のテストケースが通過
  - Issue 一覧レンダリング（#番号・タイトル・ラベル色・アサイニーアバター・更新日時）
  - gh CLI 未対応時のエラー表示
  - Issue 0 件時の空状態表示
  - 無限スクロール（IntersectionObserver のモック）
  - worktree 紐づきインジケーター

### T017-T022: フィルタ・トグル・リフレッシュテスト

- **Red**: フィルタ・トグル機能が未実装
- **Green**: 以下のテストケースが通過
  - テキスト検索によるフィルタリング
  - ラベルクリックによるフィルタリング
  - open/closed トグル切替
  - リフレッシュボタン

### T023-T027: Issue 詳細ビューテスト

- **Red**: 詳細ビューのナビゲーション・レンダリングが未実装
- **Green**: 以下のテストケースが通過
  - Issue クリックで詳細ビューに遷移
  - 戻るボタンで一覧に戻る（フィルタ保持）
  - メタ情報ヘッダーの表示
  - 本文の GFM Markdown レンダリング
  - spec ラベル付き Issue で IssueSpecPanel ビューに切替

### T028-T033: フルフロー連携テスト

- **Red**: AgentLaunchForm に Issue プリフィル機能が未実装
- **Green**: 以下のテストケースが通過
  - 「Work on this」クリックで AgentLaunchForm 起動イベント発火
  - プリフィル値の検証（prefix 推定: bug → bugfix/, enhancement → feature/）
  - 紐づき worktree 存在時に「Switch to worktree」ボタン表示
  - 「Open in GitHub」ボタンで外部ブラウザ起動

## 実行ログ（要約）

- **バックエンド**: cargo test 194 passed, 0 failed — clippy 0 warnings
- **フロントエンド**: pnpm test 335 passed (33 test files) — svelte-check 0 errors, 0 warnings
- **MarkdownRenderer**: 13 テスト（GFM 各要素 + XSS サニタイズ）全 GREEN
- **IssueListPanel**: 5 テスト（一覧レンダリング・エラー・空状態・フィルタ）全 GREEN
- **AgentLaunchForm**: 28 テスト（Issue プリフィル含む）全 GREEN — labels 型変更対応済み

## 残課題

### 解決済み（実装フェーズ後の分析で発見・修正）

- **FR-015 / FR-016 プリフィル修正**: AgentLaunchForm の Issue プリフィルでブランチ名（`{prefix}issue-{number}`）とラベルからの prefix 推定（bug → bugfix/, enhancement → feature/）が正しく動作するよう修正。テスト追加で回帰防止済み
- **FR-014 SpecIssue 表示修正**: `spec` ラベル付き Issue で IssueSpecPanel のセクション解析ビューが正しく切り替わるよう修正。テスト追加済み
- **FR-003 タブラベル修正**: タブラベルの「Issues (N)」形式で件数 N が正しく表示されるよう修正。テスト追加済み

全テスト通過確認済み（cargo test + pnpm test）
