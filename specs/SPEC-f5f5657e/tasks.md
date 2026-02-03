# タスク一覧: Docker環境統合（エージェント自動起動）

**仕様ID**: `SPEC-f5f5657e`
**作成日**: 2026-02-03

## Phase 1: Docker検出基盤

### TASK-001: dockerモジュール基盤作成

**ステータス**: 完了 ✅
**優先度**: P1
**依存**: なし

**内容**:

- [ ] `crates/gwt-core/src/docker/mod.rs` を新規作成
- [ ] `crates/gwt-core/src/lib.rs` に `pub mod docker;` を追加
- [ ] サブモジュールの宣言（detector, manager, container, command, port, error, devcontainer）

**受け入れ基準**:

- `cargo build` が成功する
- dockerモジュールがgwt-coreからエクスポートされる

---

### TASK-002: Dockerファイル検出実装

**ステータス**: 完了 ✅
**優先度**: P1
**依存**: TASK-001

**内容**:

- [ ] `crates/gwt-core/src/docker/detector.rs` を新規作成
- [ ] DockerFileType列挙型の定義（Compose, Dockerfile, DevContainer）
- [ ] detect_docker_files(worktree_path: &Path) -> Option<DockerFileType> の実装
- [ ] 検出優先順位: compose.yml > Dockerfile > .devcontainer

**受け入れ基準**:

- docker-compose.yml/compose.ymlが検出される
- Dockerfileが検出される
- .devcontainer/devcontainer.jsonが検出される
- 優先順位が正しく適用される
- T-101〜T-105 テストが通る

---

## Phase 2: コンテナ管理（起動・停止）

### TASK-003: dockerコマンドラッパー実装

**ステータス**: 完了 ✅
**優先度**: P1
**依存**: TASK-001

**内容**:

- [ ] `crates/gwt-core/src/docker/command.rs` を新規作成
- [ ] docker_available() -> bool の実装
- [ ] compose_available() -> bool の実装
- [ ] daemon_running() -> bool の実装
- [ ] try_start_daemon() -> Result<()> の実装

**受け入れ基準**:

- dockerコマンドの存在を確認できる
- docker composeの利用可能を確認できる
- デーモン起動状態を確認できる
- T-201 テストが通る

---

### TASK-004: Container情報構造体実装

**ステータス**: 完了 ✅
**優先度**: P1
**依存**: TASK-001

**内容**:

- [ ] `crates/gwt-core/src/docker/container.rs` を新規作成
- [ ] ContainerInfo構造体の定義（id, name, status, ports, services）
- [ ] ContainerStatus列挙型の定義（Running, Stopped, NotFound）

**受け入れ基準**:

- コンテナ情報を表現できる
- `cargo build` が成功する

---

### TASK-005: DockerManager実装（基本）

**ステータス**: 完了 ✅
**優先度**: P1
**依存**: TASK-002, TASK-003, TASK-004

**内容**:

- [ ] `crates/gwt-core/src/docker/manager.rs` を新規作成
- [ ] DockerManager構造体の定義（worktree_path, container_name, docker_file_type）
- [ ] new(worktree_path: &Path, docker_file_type: DockerFileType) -> Self の実装
- [ ] generate_container_name(worktree_name: &str) -> String の実装
- [ ] コンテナ名正規化（英数字とハイフンのみ）

**受け入れ基準**:

- DockerManagerが作成できる
- コンテナ名が正しく生成される（gwt-{sanitized_name}）
- T-202 テストが通る

---

### TASK-006: DockerManager実装（起動・停止）

**ステータス**: 完了 ✅
**優先度**: P1
**依存**: TASK-005

**内容**:

- [ ] start(&self) -> Result<ContainerInfo> の実装（docker compose up -d）
- [ ] stop(&self) -> Result<()> の実装（docker compose down）
- [ ] is_running(&self) -> bool の実装
- [ ] 環境変数の継承処理（ホストからコンテナへ）

**受け入れ基準**:

- docker compose up -d が実行できる
- docker compose down が実行できる
- コンテナ起動状態を確認できる
- T-203, T-204 テストが通る

---

### TASK-007: DockerManager実装（再利用・再ビルド）

**ステータス**: 完了 ✅
**優先度**: P1
**依存**: TASK-006

**内容**:

- [ ] needs_rebuild(&self) -> bool の実装（Dockerfileのタイムスタンプ比較）
- [ ] rebuild(&self) -> Result<()> の実装（docker compose build）
- [ ] 既存コンテナ再利用ロジック

**受け入れ基準**:

- 既存コンテナが再利用される
- Dockerfile更新時に再ビルドされる
- T-205, T-206 テストが通る

---

### TASK-008: DockerManager実装（コンテナ内実行）

**ステータス**: 完了 ✅
**優先度**: P1
**依存**: TASK-006

**内容**:

- [ ] run_in_container(&self, command: &str, args: &[String]) -> Result<()> の実装
- [ ] TTY対応（-it オプション）
- [ ] 作業ディレクトリ設定（-w オプション）

**受け入れ基準**:

- コンテナ内でコマンドが実行できる
- TTYが正しく接続される
- T-207 テストが通る

---

## Phase 3: ポート競合解決

### TASK-009: PortAllocator実装

**ステータス**: 完了 ✅
**優先度**: P1
**依存**: TASK-001

**内容**:

- [ ] `crates/gwt-core/src/docker/port.rs` を新規作成
- [ ] PortAllocator構造体の定義
- [ ] new() -> Self の実装
- [ ] find_available_port(base_port: u16) -> Option<u16> の実装
- [ ] is_port_in_use(port: u16) -> bool の実装（TCPソケットでチェック）

**受け入れ基準**:

- 空きポートが検出できる
- ポート使用中を判定できる
- T-301〜T-303 テストが通る

---

## Phase 4: TUI統合

### TASK-010: Docker進捗表示画面実装

**ステータス**: 完了 ✅
**優先度**: P1
**依存**: TASK-006

**内容**:

- [ ] `crates/gwt-cli/src/tui/screens/docker_progress.rs` を新規作成
- [ ] DockerProgressState構造体の定義
- [ ] DockerStatus列挙型の定義
- [ ] render_docker_progress() の実装
- [ ] スピナーアニメーション
- [ ] ステータスメッセージ表示

**受け入れ基準**:

- 進捗画面が表示される
- スピナーがアニメーションする
- T-401 テストが通る

---

### TASK-011: サービス選択画面実装

**ステータス**: 完了 ✅
**優先度**: P1
**依存**: TASK-010

**内容**:

- [ ] `crates/gwt-cli/src/tui/screens/service_select.rs` を新規作成
- [ ] ServiceSelectState構造体の定義
- [ ] render_service_select() の実装
- [ ] 矢印キーでの選択
- [ ] Enterでの確定

**受け入れ基準**:

- サービス一覧が表示される
- キーボードで選択できる
- T-402, T-403 テストが通る

---

### TASK-012: TUI画面登録とフロー統合

**ステータス**: 未着手
**優先度**: P1
**依存**: TASK-010, TASK-011

**内容**:

- [ ] `crates/gwt-cli/src/tui/screens/mod.rs` に新画面を追加
- [ ] `crates/gwt-cli/src/tui/app.rs` にDocker起動フローを統合
- [ ] エージェント起動前のDockerチェック
- [ ] 状態遷移の実装

**受け入れ基準**:

- worktree選択後にDocker検出が行われる
- Docker起動中に進捗画面が表示される
- 複数サービス時に選択画面が表示される

---

## Phase 5: devcontainer対応

### TASK-013: devcontainer解析実装

**ステータス**: 未着手
**優先度**: P2
**依存**: TASK-002

**内容**:

- [ ] `crates/gwt-core/src/docker/devcontainer.rs` を新規作成
- [ ] DevContainerConfig構造体の定義
- [ ] load(path: &Path) -> Result<Self> の実装
- [ ] to_compose_args(&self) -> Vec<String> の実装

**受け入れ基準**:

- devcontainer.jsonが解析できる
- docker-compose形式に変換できる
- T-501〜T-503 テストが通る

---

## Phase 6: エラーハンドリング

### TASK-014: DockerError型実装

**ステータス**: 未着手
**優先度**: P2
**依存**: TASK-001

**内容**:

- [ ] `crates/gwt-core/src/docker/error.rs` を新規作成
- [ ] DockerError列挙型の定義
- [ ] is_retryable(&self) -> bool の実装
- [ ] Display/Error traitの実装

**受け入れ基準**:

- エラー種別が識別できる
- リトライ可能なエラーを判定できる

---

### TASK-015: リトライロジック実装

**ステータス**: 未着手
**優先度**: P2
**依存**: TASK-006, TASK-014

**内容**:

- [ ] DockerManagerにリトライロジックを追加
- [ ] 最大3回リトライ
- [ ] 待機時間: 2秒, 5秒

**受け入れ基準**:

- 失敗時に自動リトライされる
- 3回失敗後にエラーが返る
- T-601 テストが通る

---

## 統合タスク

### TASK-016: tmux/launcher.rs統合

**ステータス**: 未着手
**優先度**: P1
**依存**: TASK-008, TASK-009, TASK-012

**内容**:

- [ ] launch_agent_with_docker() 関数を追加
- [ ] 既存のlaunch_in_pane()との連携
- [ ] Docker検出→起動→エージェント実行のフロー

**受け入れ基準**:

- Docker環境でエージェントがコンテナ内で起動する
- Docker環境なしで従来通りホストで起動する

---

### TASK-017: エージェント終了時のクリーンアップ

**ステータス**: 未着手
**優先度**: P1
**依存**: TASK-016

**内容**:

- [ ] エージェント終了検知
- [ ] docker compose down の自動実行
- [ ] エラー時のクリーンアップ

**受け入れ基準**:

- エージェント終了時にコンテナが停止する
- エラー時もクリーンアップされる

---

### TASK-018: 統合テスト作成

**ステータス**: 未着手
**優先度**: P1
**依存**: TASK-016, TASK-017

**内容**:

- [ ] E2Eテストシナリオ作成
- [ ] docker-compose.ymlあり環境でのテスト
- [ ] Dockerfileのみ環境でのテスト
- [ ] Docker環境なしでのフォールバックテスト

**受け入れ基準**:

- 主要シナリオが自動テストでカバーされる
- CI環境で実行可能

---

## タスク依存関係

```text
TASK-001 (基盤)
    ├─> TASK-002 (Detector) ─┬─> TASK-005 (Manager基本) ─> TASK-006 (起動停止) ─┬─> TASK-007 (再利用)
    │                        │                                                   ├─> TASK-008 (コンテナ内実行)
    │                        └─> TASK-013 (devcontainer)                         └─> TASK-015 (リトライ)
    │                                                                                      │
    ├─> TASK-003 (Command) ─> TASK-005                                                     │
    │                                                                                      │
    ├─> TASK-004 (Container) ─> TASK-005                                                   │
    │                                                                                      │
    ├─> TASK-009 (Port)                                                                    │
    │                                                                                      │
    └─> TASK-014 (Error) ─> TASK-015                                                       │
                                                                                           │
TASK-006 ─> TASK-010 (進捗画面) ─> TASK-011 (サービス選択) ─> TASK-012 (TUI統合)             │
                                                                        │                 │
                                                                        └─────────────────┴─> TASK-016 (統合)
                                                                                                  │
                                                                        TASK-016 ─> TASK-017 (クリーンアップ) ─> TASK-018 (統合テスト)
```

## 見積もり

| フェーズ | タスク | 見積もり |
|----------|--------|----------|
| Phase 1 | TASK-001〜002 | 小 |
| Phase 2 | TASK-003〜008 | 中 |
| Phase 3 | TASK-009 | 小 |
| Phase 4 | TASK-010〜012 | 中 |
| Phase 5 | TASK-013 | 小 |
| Phase 6 | TASK-014〜015 | 小 |
| 統合 | TASK-016〜018 | 中 |

## 合計タスク数: 18
