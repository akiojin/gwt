# 実装計画: Windows 移行プロジェクトで Docker mount エラーを回避する

**仕様ID**: `SPEC-4e2f1028` | **日付**: 2026-02-13 | **仕様書**: `specs/SPEC-4e2f1028/spec.md`

## 目的

- Docker override と docker run の Git bind mount 生成を安全化し、Windows パス混在時の `too many colons` を解消する。

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-tauri/`）
- **フロントエンド**: 変更なし
- **ストレージ/外部連携**: Docker CLI / docker compose
- **テスト**: `cargo test -p gwt-tauri --lib terminal::tests::...`
- **前提**: 既存の `HOST_GIT_COMMON_DIR` / `HOST_GIT_WORKTREE_DIR` 収集ロジックは継続利用

## 実装方針

### Phase 1: Git bind mount 計画の共通化

- `terminal.rs` に mount source/target 正規化ヘルパーを追加する。
- common/worktree の包含関係を判定し、不要な重複 mount を省く。

### Phase 2: compose override と docker run への適用

- compose override を long syntax bind mount で出力する。
- docker run の `-v` 引数生成にも同じ mount 計画を適用する。

### Phase 3: 回帰防止テスト

- mixed path ケース、Windows drive-path 変換、nested worktree skip のユニットテストを追加する。
- compose exec の workdir 省略ケース（`-w` 非付与）をユニットテストで追加する。

## テスト

### バックエンド

- mount target 正規化テスト
- nested worktree mount 省略テスト
- override YAML 生成テスト（long syntax + 不正短縮記法非生成）

### フロントエンド

- 変更なし
