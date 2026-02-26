# 実装計画: SPEC-1242

## 概要

Version History のタグ並び順を Git 実装依存から切り離し、Rust 側で semver 降順に正規化する。加えて表示前に default remote からタグをベストエフォート同期し、ローカルタグが古い環境でも最新10件が表示されるようにする。

## 設計方針

### 層1: タグ整列ロジックの明示化

- `list_version_tags` が受け取ったタグ列を Rust 側で semver 降順にソートする。
- semver として解釈できないタグは後段に回し、安定した比較で順序を決定する。

### 層2: タグ同期の追加

- `list_project_versions` 実行時に default remote (`origin` 優先) へ `git fetch --tags --prune` をベストエフォートで実行する。
- タグ同期失敗時は処理を継続し、ローカルタグのみで結果を返す。

### 層3: 回帰防止テスト追加

- ソート順の検証を `version_history` ユニットテストに追加する。
- `v7.12.6` と `v7.9.0` の順序逆転が再発しないことを固定化する。
- stale clone で remote にだけ新しいタグが存在するケースを追加し、`list_project_versions` が最新タグを取り込むことを固定化する。

## 変更ファイル

| ファイル | 変更内容 |
|---|---|
| `crates/gwt-tauri/src/commands/version_history.rs` | タグ同期処理追加、タグ整列ロジック、順序検証テスト追加 |
| `specs/SPEC-1242/spec.md` | 仕様定義 |
| `specs/SPEC-1242/tasks.md` | タスク管理 |

## 検証

- `cargo test -p gwt-tauri version_history::tests::` で対象テスト実行
