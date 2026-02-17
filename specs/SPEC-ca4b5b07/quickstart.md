# クイックスタート: Issue タブ

**仕様ID**: `SPEC-ca4b5b07`

## 前提条件

- Rust stable toolchain
- Node.js + pnpm
- gh CLI インストール・認証済み (`gh auth status` で確認)
- `cd gwt-gui && pnpm install` 済み

## 開発手順

### 1. バックエンド拡張

対象ファイル: `crates/gwt-tauri/src/commands/issue.rs`

- `GitHubIssueInfo` / `GitHubLabel` の構造体を拡張
- `fetch_github_issues` に `state` パラメータ追加
- `fetch_github_issue_detail` コマンド追加
- `crates/gwt-tauri/src/app.rs` にコマンド登録

テスト実行:

```bash
cargo test -p gwt-tauri
```

### 2. メニュー追加

対象ファイル: `crates/gwt-tauri/src/menu.rs`

- `MENU_ID_GIT_ISSUES` 定数追加
- Git メニューに「Issues」項目追加

### 3. GFM Markdown ライブラリ導入

```bash
cd gwt-gui && pnpm add marked dompurify && pnpm add -D @types/dompurify
```

### 4. フロントエンド実装

新規ファイル:

- `gwt-gui/src/lib/components/MarkdownRenderer.svelte`
- `gwt-gui/src/lib/components/MarkdownRenderer.test.ts`
- `gwt-gui/src/lib/components/IssueListPanel.svelte`
- `gwt-gui/src/lib/components/IssueListPanel.test.ts`

変更ファイル:

- `gwt-gui/src/lib/types.ts` — 型定義追加
- `gwt-gui/src/App.svelte` — メニューアクション・タブ管理・フルフロー連携
- `gwt-gui/src/lib/components/MainArea.svelte` — タブレンダリング分岐
- `gwt-gui/src/lib/components/AgentLaunchForm.svelte` — Issue プリフィルロジック

テスト実行:

```bash
cd gwt-gui && pnpm test
```

### 5. 全体検証

```bash
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json
cd gwt-gui && pnpm test
```
