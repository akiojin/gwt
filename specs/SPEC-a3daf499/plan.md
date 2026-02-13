# 実装計画: 起動時更新チェック堅牢化（遅延 + 再試行）

**仕様ID**: `SPEC-a3daf499` | **日付**: 2026-02-13 | **仕様書**: `specs/SPEC-a3daf499/spec.md`

## 目的

- Windows で発生する起動時更新通知の取りこぼしを解消する。
- 全OS共通で起動時チェックを堅牢化し、起動直後の一時失敗を吸収する。
- 失敗時のユーザー体験は静かに維持しつつ、ログで障害解析可能にする。

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-tauri/`）
- **フロントエンド**: Svelte 5 + TypeScript（`gwt-gui/`）
- **ストレージ/外部連携**: GitHub Releases API（既存 `gwt_core::update::UpdateManager`）
- **テスト**: `pnpm --dir gwt-gui test`, `cargo test -p gwt-tauri`
- **前提**:
  - 既存の Tauri command 仕様 (`check_app_update`, `apply_app_update`) は変更しない。
  - 起動時イベント `app-update-state` は後方互換のため維持する。

## 実装方針

### Phase 1: 起動時更新チェックの専用ヘルパー化

- `gwt-gui/src/lib/update/startupUpdate.ts` を新規作成し、以下を実装する:
  - 初回3秒遅延
  - 失敗時3秒間隔・最大3回再試行（合計最大4試行）
  - `available` 到達時の即時コールバック
  - 中断（AbortSignal）対応
  - 失敗時は `console.warn` ログのみ出力

### Phase 2: App.svelte への統合

- `gwt-gui/src/App.svelte` の起動時更新チェック `$effect` をヘルパー利用に置換する。
- 更新通知表示ロジックを関数化し、以下の経路で再利用する:
  - 起動時ヘルパー
  - `app-update-state` イベント受信
  - 手動 `check-updates` メニュー
- 同一バージョン通知の重複防止（`lastUpdateToastVersion`）を維持する。

### Phase 3: バックエンド失敗ログ強化

- `crates/gwt-tauri/src/app.rs` の起動時バックグラウンド更新チェックで `Failed` を `tracing::warn!` 出力する。
- `crates/gwt-tauri/src/commands/update.rs` の `check_app_update` コマンドでも `Failed` を `tracing::warn!` 出力する。
- これにより起動時経路・手動経路の双方で障害トレースを残す。

## テスト

### バックエンド

- `cargo test -p gwt-tauri`

### フロントエンド

- `gwt-gui/src/lib/update/startupUpdate.test.ts` を追加し、以下を検証する:
  - 失敗後の再試行で `available` 到達
  - 再試行上限到達時の停止
  - `up_to_date` 時の即時終了
  - AbortSignal による中断
- `pnpm --dir gwt-gui test`
