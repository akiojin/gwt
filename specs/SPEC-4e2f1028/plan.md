# 実装計画: Windows 移行プロジェクトで Docker 起動失敗の可観測性を改善する

**仕様ID**: `SPEC-4e2f1028` | **日付**: 2026-02-21 | **仕様書**: `specs/SPEC-4e2f1028/spec.md`

## 目的

- Docker override と docker run の Git bind mount 生成を安全化し、Windows パス混在時の `too many colons` を解消する。
- `docker compose up` 成功直後の service 即時終了を検知し、Launch エラーへ原因ログを返す。

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-tauri/`）
- **フロントエンド**: 変更なし
- **ストレージ/外部連携**: Docker CLI / docker compose
- **テスト**: `cargo test -p gwt-tauri --lib terminal::tests::...`
- **前提**: 既存の `HOST_GIT_COMMON_DIR` / `HOST_GIT_WORKTREE_DIR` 収集ロジックは継続利用
- **前提**: Docker Compose の `ps --status running --services` と `logs` を利用可能

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

### Phase 4: compose 起動直後の service ヘルス検証

- `docker compose up` 後に選択 service が `running` か判定する。
- `running` でない場合は Launch 失敗として扱う。

### Phase 5: 起動失敗ログの可観測性改善

- service 非起動時は compose logs（tail）を取得し、Launch エラーへ付与する。
- service 名の部分一致誤検知を避ける判定ヘルパーを追加する。

## テスト

### バックエンド

- mount target 正規化テスト
- nested worktree mount 省略テスト
- override YAML 生成テスト（long syntax + 不正短縮記法非生成）
- compose service 一致判定テスト（完全一致 / 部分一致非一致）

### フロントエンド

- 変更なし
