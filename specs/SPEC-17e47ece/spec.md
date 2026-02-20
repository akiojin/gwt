# バグ修正仕様: Windows Docker Launch で `service "dev" is not running` を防止する

**仕様ID**: `SPEC-17e47ece`
**作成日**: 2026-02-20
**更新日**: 2026-02-20
**ステータス**: ドラフト
**カテゴリ**: GUI
**依存仕様**:

- `SPEC-4e2f1028`（Docker Compose 起動まわり）

**入力**: ユーザー説明: "Issue #1162: Windows 環境で Docker Launch Agent 実行時に `service \"dev\" is not running` で起動できない"

## 背景

- Launch Agent の Docker Compose 経路で `docker compose up -d` 実行時にサービス名を明示していない。
- Compose 構成によっては `exec` 対象サービス（例: `dev`）が起動状態にならず、`docker compose exec dev ...` で失敗する。

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - Docker Launch で選択サービスを確実に起動する (優先度: P0)

Windows ユーザーとして、Launch Agent の Docker 実行時に選択した Compose サービスへ確実に `exec` したい。

**独立したテスト**: `docker compose up` 引数生成が選択サービスを含むことをユニットテストで検証する。

**受け入れシナリオ**:

1. **前提条件** Launch Agent で `docker_service=dev` を選択、**操作** Docker 起動を実行、**期待結果** `docker compose up -d ... dev` が実行され、続く `exec dev` が失敗しない。
2. **前提条件** DevContainer (compose) でサービス指定あり、**操作** Docker 起動を実行、**期待結果** 同様に `up` で指定サービスが起動対象に含まれる。

---

### ユーザーストーリー 2 - 既存フローを壊さない (優先度: P1)

開発者として、既存の build/recreate フラグ挙動や `exec` 引数生成を維持したい。

**独立したテスト**: `build_docker_compose_up_args` の既存フラグテストが回帰しないことを確認する。

**受け入れシナリオ**:

1. **前提条件** `docker_build=true` または `docker_recreate=true`、**操作** 起動引数を生成、**期待結果** `--build` / `--force-recreate` が従来通り付与される。

## エッジケース

- サービス名が空文字または空白のみの場合は `up` にサービスを追加しない。
- `exec` 側の既存環境変数・workdir付与ルールは維持する。

## 要件 *(必須)*

### 機能要件

- **FR-001**: Compose 起動時、選択済みサービス名がある場合は `docker compose up` の末尾にサービス名を追加する。
- **FR-002**: DevContainer compose 起動時も同じルールを適用する。
- **FR-003**: サービス名が空の場合は既存と同じくサービス未指定の `up` 引数を生成する。

### 非機能要件

- **NFR-001**: `crates/gwt-tauri/src/commands/terminal.rs` のユニットテストで回帰防止する。

## 制約と仮定

- 修正対象は Launch Agent の Docker Compose 経路に限定する。
- Dockerfile 単独起動フロー（`docker run`）は対象外。

## 成功基準 *(必須)*

- **SC-001**: `docker compose up` 引数が選択サービスを含むことをテストで検証できる。
- **SC-002**: `cargo test -p gwt-tauri terminal::tests::build_docker_compose_up_args_build_and_recreate_flags` が通過する。
