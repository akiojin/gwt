# 実装計画: 設定画面フォントファミリー選択

**仕様ID**: `SPEC-7b36bdc7` | **日付**: 2026-02-20 | **仕様書**: `specs/SPEC-7b36bdc7/spec.md`

## 目的

- Settings の Appearance タブで UI / Terminal のフォントファミリーを選択可能にする
- フォント変更を保存前に即時プレビューし、未保存 Close 時は保存済み値へ戻す
- 保存したフォント設定を Rust 設定モデルに永続化し、次回起動時に初期反映する

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-core/`, `crates/gwt-tauri/`）
- **フロントエンド**: Svelte 5 + TypeScript（`gwt-gui/`）
- **ストレージ/外部連携**: `~/.gwt/config.toml`（settings 永続化）
- **テスト**: `cargo test`, `pnpm test`, `pnpm exec playwright test`
- **前提**: 既存の `ui_font_size` / `terminal_font_size` のプレビュー・保存挙動を維持する

## 実装方針

### Phase 1: 設定モデル拡張（Rust）

- `AppearanceSettings` に `ui_font_family` / `terminal_font_family` を追加
- 既定フォント値と正規化関数を追加し、既存設定との後方互換を維持
- Tauri 側 `SettingsData` の DTO 変換に新フィールドを追加

### Phase 2: 設定UIと反映（Svelte）

- `SettingsPanel.svelte` にフォントプリセット選択 UI を追加
- 選択時に `--ui-font-family` / `--terminal-font-family` と terminal event を即時反映
- Save で設定永続化、Close で保存済みフォントへロールバック
- `App.svelte` / `main.ts` で設定更新イベントと起動時復元を対応

### Phase 3: テストと検証

- Unit: SettingsPanel と TerminalView の既存テストを拡張
- Backend: settings 変換・既定値のテストを追加
- E2E: 設定画面でフォント変更保存・Closeロールバックを Playwright で確認

## テスト

### バックエンド

- `cargo test -p gwt-core test_appearance_ -- --test-threads=1`
- `cargo test -p gwt-tauri test_settings_data_`

### フロントエンド

- `pnpm test src/lib/components/SettingsPanel.test.ts src/lib/terminal/TerminalView.test.ts`
- `pnpm exec svelte-check --tsconfig ./tsconfig.json`
- `pnpm exec playwright test e2e/windows-shell-selection.spec.ts --grep \"font\"`
