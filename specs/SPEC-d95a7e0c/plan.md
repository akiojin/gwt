# 実装計画: URLクリック時の外部ブラウザ起動統一

**仕様ID**: `SPEC-d95a7e0c` | **日付**: 2026-02-20 | **仕様書**: `specs/SPEC-d95a7e0c/spec.md`

## 目的

- gwt 全画面で URL クリック時の挙動を統一し、`http/https` を外部ブラウザで開く。
- URL 起動ロジックの重複実装を削減し、安全なスキーム制限を導入する。

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-tauri/`）
- **フロントエンド**: Svelte 5 + TypeScript（`gwt-gui/`）
- **ストレージ/外部連携**: Tauri Shell Plugin（`@tauri-apps/plugin-shell`）
- **テスト**: Vitest（`gwt-gui/src/lib/**/*.test.ts`）
- **前提**:
  - URL スキーム許可は `http/https` のみ
  - Terminal には xterm addon-web-links を使用

## 実装方針

### Phase 1: URL外部起動基盤

- `gwt-gui/src/lib/openExternalUrl.ts` を追加し、以下を実装する。
  - `isAllowedExternalHttpUrl(raw)`
  - `openExternalUrl(raw)`（`plugin-shell open` 優先、失敗時フォールバック）
- `openExternalUrl` のユニットテストを追加する。

### Phase 2: UI/Terminal の統一適用

- `gwt-gui/src/App.svelte` にグローバルリンククリック委譲を追加し、`a[href]` の `http/https` を `openExternalUrl` に集約する。
- `gwt-gui/src/lib/terminal/TerminalView.svelte` の `WebLinksAddon` にクリックハンドラを渡し、Terminal URL クリックを `openExternalUrl` に統一する。
- `gwt-gui/src/lib/components/IssueListPanel.svelte` / `gwt-gui/src/lib/components/WorktreeSummaryPanel.svelte` の個別 `window.open` / plugin-shell 呼び出しを共通化する。

### Phase 3: Tauri設定と回帰テスト

- `crates/gwt-tauri/Cargo.toml` に `tauri-plugin-shell` を追加する。
- `crates/gwt-tauri/src/app.rs` で `tauri_plugin_shell::init()` を登録する。
- `crates/gwt-tauri/capabilities/default.json` に `shell:allow-open` を追加する。
- 既存テスト更新（`TerminalView.test.ts` / `WorktreeSummaryPanel.test.ts`）で挙動を検証する。

## テスト

### バックエンド

- 追加ユニットテストなし（設定変更のみ）。

### フロントエンド

- `gwt-gui/src/lib/openExternalUrl.test.ts`
  - `http/https` 許可、非許可スキーム拒否
  - plugin-shell 成功時の呼び出し
  - plugin-shell 失敗時フォールバック
- `gwt-gui/src/lib/terminal/TerminalView.test.ts`
  - WebLinksAddon のクリックハンドラから外部起動関数が呼ばれること
- `gwt-gui/src/lib/components/WorktreeSummaryPanel.test.ts`
  - Workflow URL オープンが共通オープナー経由で呼ばれること
