# 実装計画: Windows 移行プロジェクトの Docker 起動でポート競合を回避する

**仕様ID**: `SPEC-1161b0a1` | **日付**: 2026-02-20 | **仕様書**: `specs/SPEC-1161b0a1/spec.md`

## 目的

- compose env マージ時のポート巻き戻りを防ぎ、`5432` 競合で Docker 起動が失敗するケースを解消する。

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-tauri/`）
- **フロントエンド**: 変更なし
- **外部連携**: Docker / docker compose
- **テスト**: `cargo test -p gwt-tauri <test-name>`

## 実装方針

### Phase 1: RED テスト追加

- `merge_compose_env_for_docker` に対して、使用中ポートへ上書きしないテストを追加する。

### Phase 2: マージロジック修正

- compose env マージ時に、既存/新規値がポート番号で新規値が使用中の場合は既存値を保持するガードを追加する。

### Phase 3: 回帰確認

- 既存の compose env マージ関連テストと追加テストを実行し、回帰がないことを確認する。

## テスト

### バックエンド

- 使用中ポートへの上書き抑止テスト
- 既存の compose env マージテスト（非ポート env 継承）

### フロントエンド

- 変更なし
