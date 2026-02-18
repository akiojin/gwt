# 実装計画: メニュー無反応の受信経路修復と可観測性強化

**仕様ID**: `SPEC-9c1e4d2f` | **日付**: 2026-02-18 | **仕様書**: `specs/SPEC-9c1e4d2f/spec.md`

## 目的

- Aboutを含む `menu-action` 無反応時に無言失敗を排除する。
- フロント/バック双方でメニューイベントの断面観測を可能にする。
- 回避策ではなく既存受信経路を修復し、原因特定時間を短縮する。

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2 (`crates/gwt-tauri`)
- **フロントエンド**: Svelte 5 + TypeScript (`gwt-gui`)
- **対象経路**:
  - Rust `on_menu_event` → `emit_menu_action`
  - Frontend `setupMenuActionListener` → `handleMenuAction`
- **テスト**: `pnpm --dir gwt-gui test`, `cargo test -p gwt-tauri`

## 実装方針

### Phase 1: 失敗を握りつぶさないフロント経路

- `gwt-gui/src/lib/menuAction.ts`
  - `webviewWindow` import/listen失敗時に文脈付きエラーへ変換して再送出
- `gwt-gui/src/App.svelte`
  - メニューリスナー初期化 `catch` を空処理から `console.error + appError設定` へ変更
  - 成功時の登録ログを追加

### Phase 2: Rust送信断面ログの追加

- `crates/gwt-tauri/src/app.rs`
  - `on_menu_event` 受信IDログ
  - `emit_menu_action` の target/action/result ログ
  - target未解決時のwarnログ

### Phase 3: TDDで検証

- 先に `gwt-gui/src/lib/menuAction.test.ts` へ失敗系テストを追加（RED）
- 実装後に同テストをGREEN化
- 既存Rustテストと対象フロントテストを実行

## テスト

### Frontend

- `gwt-gui/src/lib/menuAction.test.ts`
  - window-scoped listener登録
  - payload転送
  - unlisten返却
  - listen失敗時のエラー伝播（新規）

### Backend

- `cargo test -p gwt-tauri menu_action_from_id_maps`

### 手動確認

- macOS配布バイナリで `About` / `Preferences` / `Check for Updates` をクリック
- 不具合時はコンソール/バックエンドログで停止断面を特定できることを確認
