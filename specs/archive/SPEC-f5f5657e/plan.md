# 実装計画: Docker環境統合（エージェント自動起動）

**仕様ID**: `SPEC-f5f5657e`
**計画作成日**: 2026-02-03

## フェーズ概要

| フェーズ | 内容 | 優先度 |
|---------|------|--------|
| Phase 1 | Docker検出基盤 | P1 |
| Phase 2 | コンテナ管理（起動・停止） | P1 |
| Phase 3 | ポート競合解決 | P1 |
| Phase 4 | TUI統合 | P1 |
| Phase 5 | devcontainer対応 | P2 |
| Phase 6 | エラーハンドリング | P2 |

## Phase 1: Docker検出基盤

### 目的

Docker関連ファイルの検出機能を実装する。設定ファイルは不要（Dockerファイル存在で自動有効）。

### 変更対象ファイル

| ファイル | 変更内容 |
|----------|----------|
| `crates/gwt-core/src/docker/mod.rs` | 新規: Dockerモジュールの定義 |
| `crates/gwt-core/src/docker/detector.rs` | 新規: Dockerファイル検出ロジック |
| `crates/gwt-core/src/lib.rs` | 変更: dockerモジュールの追加 |

### 実装詳細

#### docker/detector.rs

DockerFileType: 検出されたDockerファイルの種類
- Compose(PathBuf): docker-compose.yml or compose.yml
- Dockerfile(PathBuf): Dockerfile
- DevContainer(PathBuf): .devcontainer/devcontainer.json

検出関数: detect_docker_files(worktree_path: &Path) -> Option<DockerFileType>

### テスト

- T-101: docker-compose.ymlの検出テスト
- T-102: compose.ymlの検出テスト
- T-103: Dockerfileの検出テスト
- T-104: .devcontainerの検出テスト
- T-105: 複数ファイル存在時の優先順位テスト

---

## Phase 2: コンテナ管理（起動・停止）

### 目的

Dockerコンテナのライフサイクル管理（起動・停止・再利用）を実装する。

### 変更対象ファイル

| ファイル | 変更内容 |
|----------|----------|
| `crates/gwt-core/src/docker/manager.rs` | 新規: DockerManager実装 |
| `crates/gwt-core/src/docker/container.rs` | 新規: Container情報構造体 |
| `crates/gwt-core/src/docker/command.rs` | 新規: dockerコマンド実行ラッパー |

### 実装詳細

#### docker/manager.rs

DockerManager構造体:
- worktree_path: PathBuf
- container_name: String
- docker_file_type: DockerFileType

メソッド:
- new(worktree_path: &Path, docker_file_type: DockerFileType) -> Self
- generate_container_name(worktree_name: &str) -> String (gwt-{sanitized_worktree_name})
- start(&self) -> Result<ContainerInfo> (compose up -d)
- stop(&self) -> Result<()> (compose down)
- is_running(&self) -> bool (既存コンテナ確認)
- needs_rebuild(&self) -> bool (イメージ更新確認)
- rebuild(&self) -> Result<()> (再ビルド)
- run_in_container(&self, command: &str, args: &[String]) -> Result<()> (コンテナ内でコマンド実行)

#### docker/command.rs

- docker_available() -> bool (dockerコマンドの存在確認)
- compose_available() -> bool (docker compose利用可能確認)
- daemon_running() -> bool (Dockerデーモン起動確認)
- try_start_daemon() -> Result<()> (デーモン起動試行)

### テスト

- T-201: コンテナ名生成テスト（正規化）
- T-202: docker compose up実行テスト
- T-203: docker compose down実行テスト
- T-204: 既存コンテナ再利用テスト
- T-205: イメージ更新検知テスト
- T-206: docker run_in_container実行テスト
- T-207: dockerコマンド存在確認テスト

---

## Phase 3: ポート競合解決

### 目的

複数worktreeでのポート競合を検出し、動的にポートを割り当てる機能を実装する。

### 変更対象ファイル

| ファイル | 変更内容 |
|----------|----------|
| `crates/gwt-core/src/docker/port.rs` | 新規: PortAllocator実装 |

### 実装詳細

#### docker/port.rs

PortRange構造体:
- start: u16
- end: u16

PortAllocator構造体:
- port_env_vars: Vec<String>

メソッド:
- new() -> Self
- find_available_port(&self, env_name: &str) -> Option<u16> (空きポート検索)
- is_port_in_use(port: u16) -> bool (ポート使用中確認)
- allocate_all(&self) -> HashMap<String, String> (環境変数として割り当て結果を返す)

### テスト

- T-301: 空きポート検索テスト
- T-302: ポート使用中確認テスト
- T-303: 範囲内割り当てテスト
- T-304: 競合時の別ポート割り当てテスト

---

## Phase 4: TUI統合

### 目的

Docker起動の進捗表示とサービス選択UIを実装する。

### 変更対象ファイル

| ファイル | 変更内容 |
|----------|----------|
| `crates/gwt-cli/src/tui/screens/docker_progress.rs` | 新規: Docker進捗表示画面 |
| `crates/gwt-cli/src/tui/screens/service_select.rs` | 新規: サービス選択画面 |
| `crates/gwt-cli/src/tui/screens/mod.rs` | 変更: 新画面の追加 |
| `crates/gwt-cli/src/tui/app.rs` | 変更: Docker起動フローの統合 |

### 実装詳細

#### docker_progress.rs

DockerProgressState構造体:
- status: DockerStatus
- message: String
- spinner_frame: usize

DockerStatus列挙型:
- DetectingFiles
- BuildingImage
- StartingContainer
- WaitingForServices
- Ready
- Failed(String)

render_docker_progress(state: &DockerProgressState, frame: &mut Frame, area: Rect)

#### service_select.rs

ServiceSelectState構造体:
- services: Vec<String>
- selected: usize

render_service_select(state: &ServiceSelectState, frame: &mut Frame, area: Rect)

### テスト

- T-401: 進捗表示レンダリングテスト
- T-402: サービス選択UIテスト
- T-403: キーボード操作テスト

---

## Phase 5: devcontainer対応

### 目的

.devcontainer/devcontainer.jsonの解析とdocker-composeへの変換を実装する。

### 変更対象ファイル

| ファイル | 変更内容 |
|----------|----------|
| `crates/gwt-core/src/docker/devcontainer.rs` | 新規: devcontainer解析 |

### 実装詳細

#### devcontainer.rs

DevContainerConfig構造体 (Deserialize):
- name: Option<String>
- docker_compose_file: Option<String>
- dockerfile: Option<String>
- build: Option<BuildConfig>
- forward_ports: Option<Vec<u16>>

メソッド:
- load(path: &Path) -> Result<Self>
- to_compose_args(&self) -> Vec<String> (docker-compose相当の設定に変換)

### テスト

- T-501: devcontainer.json解析テスト
- T-502: dockerComposeFile指定時のテスト
- T-503: Dockerfile指定時のテスト

---

## Phase 6: エラーハンドリング

### 目的

エラー時のリトライロジックを実装する。ログは既存のgwtログ機構に統合（tracingクレート使用）。

### 変更対象ファイル

| ファイル | 変更内容 |
|----------|----------|
| `crates/gwt-core/src/docker/error.rs` | 新規: Dockerエラー型 |

### 実装詳細

#### error.rs

DockerError列挙型:
- DaemonNotRunning
- BuildFailed(String)
- StartFailed(String)
- RunInContainerFailed(String)
- PortConflict(u16)
- Timeout

メソッド:
- is_retryable(&self) -> bool

#### ログ出力

既存のtracingクレートを使用してカテゴリ`docker`で出力。
別途ログファイルは作成しない。

### テスト

- T-601: リトライロジックテスト
- T-602: エラー種別判定テスト

---

## 既存コードへの統合

### tmux/launcher.rsへの変更

エージェント起動前にDockerを起動し、コンテナ内でエージェントを実行するようにフローを変更。

launch_agent_with_docker関数を追加:
1. Dockerファイル検出
2. 検出された場合:
   - DockerManager作成
   - コンテナ起動 (start)
   - コンテナ内でエージェント起動 (docker run_in_container -it {container} {agent_command})
3. 検出されない場合:
   - 従来のホスト起動

---

## 依存関係グラフ

```text
Phase 1 ─┬─> Phase 2 ─┬─> Phase 4
         │            │
         └─> Phase 3 ─┘
         │
         └─> Phase 5
         │
         └─> Phase 6
```

- Phase 1（検出）が基盤
- Phase 2（コンテナ管理）とPhase 3（ポート）はPhase 1に依存
- Phase 4（TUI）はPhase 2, 3に依存
- Phase 5, 6は並行して実装可能

---

## リスクと軽減策

| リスク | 影響 | 軽減策 |
|--------|------|--------|
| Docker環境の多様性 | 中 | Docker Desktop/Engine両対応、エラー時ホストへフォールバック |
| ポート競合の複雑さ | 低 | compose設定を尊重、gwt側での動的割り当ては最小限 |
| devcontainer仕様の複雑さ | 中 | 主要フィールドのみ対応、未対応はcomposeとして処理 |
| コンテナ起動の遅延 | 中 | 既存コンテナ再利用、並列起動の検討 |
