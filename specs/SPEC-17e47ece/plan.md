# 実装計画: Windows Docker Launch で `service "dev" is not running` を防止する

**仕様ID**: `SPEC-17e47ece` | **日付**: 2026-02-20 | **仕様書**: `specs/SPEC-17e47ece/spec.md`

## 目的

- Docker Compose Launch で `exec` 対象サービスが未起動になる経路を解消する。

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-tauri/`）
- **フロントエンド**: 変更なし
- **ストレージ/外部連携**: Docker CLI / docker compose
- **テスト**: `cargo test -p gwt-tauri terminal::tests::...`
- **前提**: Launch Agent は Compose / DevContainer(Compose) 経路で同一の `docker_compose_up` を利用する

## 実装方針

### Phase 1: テスト先行（TDD）

- `build_docker_compose_up_args` のテストに「サービス指定時は末尾にサービス名を付与」を追加する。

### Phase 2: Compose 起動引数の修正

- `build_docker_compose_up_args` をサービス名付きで生成できるよう拡張する。
- `docker_compose_up` 呼び出し時に選択サービスを渡す。
- Compose / DevContainer(Compose) の両経路に適用する。

### Phase 3: 検証

- 追加テストと既存関連テストを実行し、回帰がないことを確認する。

## テスト

### バックエンド

- `build_docker_compose_up_args` の既存フラグテスト
- 同テスト内でサービス指定ケースの追加検証

### フロントエンド

- 変更なし
