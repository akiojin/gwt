# 実装計画: v7.11.0 起動不能（Issue #1219）

**仕様ID**: `SPEC-434cef4e` | **日付**: 2026-02-24 | **仕様書**: `specs/SPEC-434cef4e/spec.md`

## 目的

- 配布バンドルのメイン実行バイナリが `voice_eval` に誤置換される不具合を防ぎ、インストール後に gwt が起動できる状態へ戻す。

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-tauri/`）
- **フロントエンド**: 変更なし
- **CI/配布**: GitHub Actions release workflow（`.github/workflows/release.yml`）
- **テスト**: `cargo metadata`、必要に応じて `cargo check -p gwt-tauri --bin ...`
- **前提**: `voice_eval` は開発向けの補助バイナリとして継続利用する

## 実装方針

### Phase 1: Cargo バイナリ定義の固定

- `crates/gwt-tauri/Cargo.toml` に `default-run = "gwt-tauri"` を追加する。
- `[[bin]]` を明示して `gwt-tauri`（GUI本体）と `voice_eval`（補助）を分離定義する。

### Phase 2: リリースビルドコマンドの固定

- `.github/workflows/release.yml` の Tauri ビルドコマンドを `cargo tauri build -- --bin gwt-tauri` に変更する。
- 将来的な追加バイナリが存在しても配布エントリポイントが変化しないことを担保する。

### Phase 3: 仕様/検証更新

- `specs/SPEC-434cef4e/spec.md` と `tasks.md` に原因・対策・検証手順を残す。
- `cargo metadata` で `default_run` と target 定義を確認する。

## テスト

### バックエンド/ビルド設定

- `cargo metadata --format-version=1 --no-deps` で `gwt-tauri` パッケージの `default_run` が `gwt-tauri` であることを確認。
- `cargo check -p gwt-tauri --bin gwt-tauri --bin voice_eval` を実行し、両バイナリ定義が成立することを確認。

### フロントエンド

- 変更なし（対象外）。
