# 実装計画: gwt 起動時に前回のWindowを復元する

**仕様ID**: `SPEC-8f9b2d01` | **日付**: 2026-02-17 | **仕様書**: `specs/SPEC-8f9b2d01/spec.md`

## 目的

- gwt起動時に前回開いていたWindowを復元し、マルチウィンドウ運用の再開コストを低減する
- 破損データや同時起動時の競合でも壊れない best-effort 再現性を担保する

## 技術コンテキスト

- **バックエンド**: Tauri v2 (`crates/gwt-tauri`)
  - ウィンドウラベル取得・ウィンドウ作成用コマンドを利用
- **フロントエンド**: Svelte 5 + TypeScript (`gwt-gui`)
  - 起動時副作用でセッション読込/復元を実施
- **ストレージ**: localStorage（`gwt.windowSessions.v1`）

## 実装方針

### Phase 1: 画面復元キーの構造化

- `gwt-gui/src/lib/windowSessions.ts` に window単位の永続化フォーマットを整理し、label単位でprojectPathを保持
- `load/get/upsert/remove/persist` を使い、sanitize/重複排除/invalid値除去を実施
- localStorage不可時のフォールバックを追加

### Phase 2: ウィンドウ起動時復元フロー（フロント）

- `gwt-gui/src/App.svelte` で起動時に `get_current_window_label` を呼び出し、現在labelを特定
- 保存セッションから現在label以外のWindowを走査し、`open_gwt_window` を呼ぶ
- `gwt.windowSessions.restoreLeader.v1` をTTL付きロックで管理し、同時起動時の重複復元を防止
- 自ウィンドウは保存されたprojectPathがあれば `open_project` を実行して復元

### Phase 3: ウィンドウセッション保存整合

- `open_project` 成功時に `updateWindowSession` で現在WindowのprojectPathを保存
- `close-project` 時に `updateWindowSession(null)` を実行し、保存エントリを削除
- `restore` 中に open 失敗したセッションは削除して再試行コストを低減

### Phase 4: TDD・検証

- `gwt-gui/src/lib/windowSessions.test.ts` で sanitize/重複排除/upsert/remove の振る舞いを検証
- `crates/gwt-tauri/src/commands/window.rs` のテスト（`normalize_window_label` fallback）を確認
- `cargo test` / `cargo test -p gwt-tauri` でRust側検証
- `pnpm -C gwt-gui check` / `pnpm -C gwt-gui test` の実行可否を確認（未導入環境は手動実行結果を残す）
