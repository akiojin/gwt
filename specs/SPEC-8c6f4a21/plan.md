# 実装計画: Windows での外部プロセス実行時コンソール点滅抑止

**仕様ID**: `SPEC-8c6f4a21` | **日付**: 2026-02-13 | **仕様書**: `specs/SPEC-8c6f4a21/spec.md`

## 目的

- Windows で外部コマンド実行時に一瞬表示されるコンソールウィンドウを抑止する。
- 外部コマンド実行経路を共通化し、再発防止できる形にする。

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-core/`, `crates/gwt-tauri/`）
- **フロントエンド**: Svelte 5 + TypeScript（変更なし）
- **ストレージ/外部連携**: Git/Docker/gh/Agent CLI（継続利用）
- **テスト**: `cargo check`, `cargo test`, `cargo test -p gwt-core`
- **前提**: Windows では `std::os::windows::process::CommandExt::creation_flags` が利用可能

## 実装方針

### Phase 1: 共通起動ヘルパー導入

- `crates/gwt-core/src/process.rs` を追加。
- `command()` / `git_command()` / `tokio_command()` と no-window 設定関数を実装し、Windows では `CREATE_NO_WINDOW` を適用する。
- `crates/gwt-core/src/lib.rs` で `process` モジュールを公開する。

### Phase 2: 外部コマンド実行経路の統一

- `gwt-core` / `gwt-tauri` の本番ソースで `Command::new(...)` / `tokio::process::Command::new(...)` を `gwt_core::process::command()` / `tokio_command()` に置換する。
- 既存の引数・環境変数・エラーハンドリングは変更しない。

### Phase 3: 再発防止と回帰確認

- `crates/gwt-core/tests/no_direct_git_command.rs` で、ワークスペース Rust コードに直接 `Command::new(...)` 系が混入していないことを検証する。
- `process.rs` にユニットテストを追加して API の基本動作を検証する。
- `cargo check` と `cargo test` を実行して回帰を確認する。

## テスト

### バックエンド

- `cargo check -q`
- `cargo test -q`
- `cargo test -q -p gwt-core`
- `rg` で `process.rs` / ガードテスト以外の `Command::new(...)` がワークスペース Rust コードに残っていないことを確認

### フロントエンド

- 変更なし（対象外）
