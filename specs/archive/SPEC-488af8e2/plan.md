# 実装計画: gwt GUI Docker Compose 統合（起動ウィザード + Quick Start）

**仕様ID**: `SPEC-488af8e2` | **日付**: 2026-02-09 | **仕様書**: `specs/SPEC-488af8e2/spec.md`

## 概要

本実装では、GUI 版のエージェント起動に Docker Compose 統合を追加し、TUI 相当の「起動前に Compose を起動し、必要なら service を選択してコンテナ内で起動する」体験を復元する。

- 起動ウィザードで Host/Docker + Service + Build/Recreate/Keep を指定可能にする
- Compose を検出した場合のみ Docker UI を表示し、`docker.force_host` が有効なら常にホスト起動にする
- Launch/Exit 時に docker 設定を `ToolSessionEntry` に保存し、Quick Start で復元する

## 技術コンテキスト

- **GUI**: Tauri v2 + Svelte 5 + Vite（`gwt-gui/`）
- **Backend**: Rust（`crates/gwt-tauri/`）
- **Core**: Rust（`crates/gwt-core/`）
- **Docker 検出**: `gwt_core::docker::detect_docker_files`（`crates/gwt-core/src/docker/`）
- **履歴**: `gwt_core::config::ToolSessionEntry`（`crates/gwt-core/src/config/ts_session.rs`）
- **PTY**: portable-pty + xterm.js

## 原則チェック

- Spec-first/TDD（仕様 → テスト → 実装）
- GUI 表示文言は英語のみ
- 設定/履歴ファイル読み込み時にディスク副作用を書かない（Save/Launch 明示時のみ書き込み）
- 既存コードの改修を優先し、必要最小限の新規追加に留める

## 実装方針（決定事項）

### 1) Docker Context 検出（Backend）

- `detect_docker_context(projectPath, branch)` を追加し、以下を返す:
  - worktree path（存在すれば）
  - Compose 検出有無（本仕様は compose のみ）
  - compose services（YAML を parse して抽出、docker コマンド不要）
  - docker/compose/daemon の可用性（`gwt_core::docker::command`）
  - `docker.force_host`（`gwt_core::config::Settings::load`）

### 2) 起動ウィザード UI（Frontend）

- `AgentLaunchForm` に Docker セクションを追加する（Compose 検出時のみ表示）
- UI は以下の入力を持つ:
  - Runtime: HostOS / Docker
  - Service: compose services から選択（複数時）
  - Build/Recreate/Keep（toggle）
- `docker.force_host` が true の場合は Docker セクションを非表示にする

### 3) 起動実行（Backend: launch_agent）

- `LaunchAgentRequest` に docker fields を追加する:
  - `dockerService`, `dockerForceHost`, `dockerRecreate`, `dockerBuild`, `dockerKeep`
- Docker が選択され、Compose が検出できる場合:
  - `COMPOSE_PROJECT_NAME=gwt-{sanitized_branch}` を設定して `docker compose up -d` を実行
  - `docker compose exec` を PTY 起動コマンドとして使用し、コンテナ内でエージェントを起動する
  - `Keep=false` の場合は、エージェント終了後に `docker compose down` をベストエフォートで実行する
- Docker が使えない/検出できない場合はエラーにして UI に表示する（ホスト起動はユーザーが明示選択する）

### 4) 履歴/Quick Start（Backend + Frontend）

- `ToolSessionEntry` に docker fields を保存する（Launch 時 + Exit 時の追記）
- Quick Start の Launch リクエストに docker fields を含めて復元する

## テスト戦略

- Rust:
  - Compose service 抽出（YAML parse）のユニットテスト
  - `launch_agent` の docker compose 引数生成のユニットテスト（build/recreate/keep）
- Frontend:
  - `cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json`
- 最終ゲート:
  - `cargo test`
  - `cargo clippy --all-targets --all-features -- -D warnings`

## リスクと緩和策

- **Docker/Compose の環境差**: docker compose の有無、daemon 状態
  - **緩和策**: 事前検出を返し、UI に明確なエラーを表示（クラッシュしない）
- **compose の実体は複雑**: override/複数ファイルなど
  - **緩和策**: 本仕様では単一 compose ファイルの services 抽出のみを扱う（高度な compose 解決は範囲外）
