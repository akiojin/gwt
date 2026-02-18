# 機能仕様: gwt GUI Docker Compose 統合（起動ウィザード + Quick Start）

**仕様ID**: `SPEC-488af8e2`
**作成日**: 2026-02-09
**ステータス**: ドラフト
**カテゴリ**: GUI

**依存仕様**:

- `SPEC-86bb4e7c`（GUI: ブランチフィルター/起動ウィザード/Profiles）
- `SPEC-90217e33`（GUI: Quick Start / 起動オプション拡張）

**参照仕様（移植元 / Porting）**:

- `SPEC-f5f5657e`（Docker環境統合）

**入力**: ユーザー報告: 「TUI では Worktree 選択後に Docker 起動ウィザードがあったが、GUI にない。Quick Start でも同じ起動条件を復元したい」

## 背景

- gwt は Worktree ごとに開発環境（Docker compose）を分離して起動できることが重要。
- GUI 版ではエージェント起動機能が整備されつつあるが、Docker 起動の導線と Quick Start 復元が未実装。
- 既存設定/履歴ファイルは、読み込みだけで初期化（上書き/空保存/変換）されてはならない（Save/Launch 明示時のみ書き込み）。

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - Compose 検出と起動ウィザード (優先度: P0)

開発者として、worktree に `docker-compose.yml`/`compose.yml` がある場合は GUI の起動ウィザードで Host/Docker を選択でき、Docker を選択した場合は compose を起動した上でコンテナ内でエージェントを実行したい。

**独立したテスト**: docker-compose.yml を含む既存 worktree を選択し、GUI の Launch Agent で Docker を選択して起動し、起動後にコンテナが立ち上がり、ターミナル上でエージェントが動作することを確認。

**受け入れシナリオ**:

1. **前提条件** 対象 worktree に `docker-compose.yml` が存在、**操作** 起動ウィザードを開く、**期待結果** `Runtime`（Host/Docker）の選択UIが表示される
2. **前提条件** Compose に複数サービスがある、**操作** Docker を選択、**期待結果** `Service` の選択UIが表示され、1つがデフォルト選択される
3. **前提条件** Docker を選択し `Build=false` `Recreate=false` で Launch、**操作** Launch、**期待結果** `docker compose up -d --no-build` が実行され、`docker compose exec` でエージェントが起動する
4. **前提条件** Docker を選択し `Build=true` で Launch、**操作** Launch、**期待結果** `docker compose up -d --build` が実行される
5. **前提条件** Docker を選択し `Recreate=true` で Launch、**操作** Launch、**期待結果** `docker compose up -d --force-recreate` が実行される
6. **前提条件** Docker を選択し `Keep=false` で Launch、**操作** エージェント終了、**期待結果** `docker compose down` が実行される（ベストエフォート）
7. **前提条件** Docker を選択し `Keep=true` で Launch、**操作** エージェント終了、**期待結果** `docker compose down` は実行されない

---

### ユーザーストーリー 2 - Quick Start で Docker 選択を復元 (優先度: P0)

開発者として、Quick Start から起動するときに、前回起動時の Docker 選択（Host/Docker + Service + Keep/Recreate/Build）が復元され、同じ条件で即時起動できるようにしたい。

**独立したテスト**: Docker 起動で履歴を残し、Summary の Quick Start Continue/New を押して同じ Docker 設定で起動されることを確認。

**受け入れシナリオ**:

1. **前提条件** Docker 起動の履歴が存在、**操作** Summary の Quick Start で Continue を押す、**期待結果** 前回と同じ Docker 設定で起動し、ウィザードを表示しない
2. **前提条件** Host 起動（Docker 強制スキップ）の履歴が存在、**操作** Quick Start で Continue/New を押す、**期待結果** Docker を起動せずホストで起動する

---

### ユーザーストーリー 3 - docker.force_host による強制ホスト起動 (優先度: P1)

開発者として、`docker.force_host=true`（または `GWT_DOCKER_FORCE_HOST=true`）の場合は Docker 関連 UI を出さず、常にホストでエージェントを起動したい。

**独立したテスト**: Settings で `docker_force_host=true` にして起動ウィザードを開き、Docker UI が出ないことを確認。

**受け入れシナリオ**:

1. **前提条件** `docker.force_host=true`、**操作** Launch Agent を開く、**期待結果** Runtime/Docker のUIが表示されない
2. **前提条件** `docker.force_host=true`、**操作** Launch、**期待結果** Docker を実行せずホストで起動する

## エッジケース

- Docker を選択して Launch したが `docker`/`docker compose` が利用できない場合、UI はクラッシュせず、エラーとして表示して起動を中断する
- Docker を選択して Launch したが Docker デーモンが停止している場合、`try_start_daemon` を試行し、起動できない場合はエラー表示して起動を中断する
- Compose 検出が無い場合は Docker UI を表示せず、従来通りホストで起動する
- 既存設定/履歴ファイルは読み込みだけで変更しない（Save/Launch 明示時のみ書き込み）

## 要件 *(必須)*

### 機能要件

#### Detection / Wizard

- **FR-001**: worktree に `docker-compose.yml`/`docker-compose.yaml`/`compose.yml`/`compose.yaml` が存在する場合、起動ウィザードに Host/Docker の選択を表示しなければ**ならない**
- **FR-002**: `docker.force_host=true` の場合、Docker 関連 UI を表示しては**ならない**
- **FR-003**: Compose が複数サービスの場合、Docker 選択時に service を選択できなければ**ならない**
- **FR-004**: Docker が利用できない場合でも UI はクラッシュしては**ならない**（エラー表示で継続）

#### Launch / History

- **FR-010**: Docker 起動時は `COMPOSE_PROJECT_NAME=gwt-{sanitized_branch}` を設定し、worktree 間で Compose を分離しなければ**ならない**
- **FR-011**: Docker 起動時は `docker compose up -d` を実行し、その後 `docker compose exec` でエージェントを起動しなければ**ならない**
- **FR-012**: 起動ウィザードの Docker 設定（service/forceHost/recreate/build/keep）は `ToolSessionEntry` に保存し、Quick Start で復元できなければ**ならない**
- **FR-013**: `Keep=false` の場合、エージェント終了後に `docker compose down` をベストエフォートで実行しなければ**ならない**
- **FR-014**: 読み込み時に履歴や設定ファイルを初期化（上書き/空保存）しては**ならない**

## インターフェース（フロント/バック間）

### Tauri Commands

- `detect_docker_context(projectPath: string, branch: string) -> DockerContext`
- `launch_agent(request: LaunchAgentRequest) -> paneId: string`（Docker フィールドを含む）

### DTO 追加（LaunchAgentRequest）

- `dockerService?: string`
- `dockerForceHost?: boolean`
- `dockerRecreate?: boolean`
- `dockerBuild?: boolean`
- `dockerKeep?: boolean`

### レスポンス型（DockerContext）

- `worktreePath?: string | null`（存在すれば）
- `fileType: "compose" | "none"`（本仕様では compose のみを扱う）
- `composeServices: string[]`
- `dockerAvailable: boolean`
- `composeAvailable: boolean`
- `daemonRunning: boolean`
- `forceHost: boolean`（Settings/env）

## 成功基準 *(必須)*

- Compose を含む worktree で GUI 起動ウィザードから Docker 起動ができる
- Quick Start で Docker 選択が復元され、同じ条件で即時起動できる
- `docker.force_host` が有効な場合に Docker UI が出ずホスト起動される

## 制約と仮定

- GUI のユーザー向け表示は英語のみ
- Dockerfile のみ / .devcontainer の統合は本仕様の範囲外（別仕様で扱う）

## 範囲外 *(必須)*

- Dockerfile のみの `docker build`/`docker run` 起動
- devcontainer.json からの compose 起動
- Compose override 自動生成（bind mount 追加等）

## 依存関係 *(該当する場合)*

- Docker Desktop / docker engine がユーザー環境で利用可能であること

## 参考資料 *(該当する場合)*

- `SPEC-f5f5657e`（移植元: Docker環境統合）
