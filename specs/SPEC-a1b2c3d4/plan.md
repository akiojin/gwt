# 実装計画: エージェント起動統計・システムモニター・About強化

**仕様ID**: `SPEC-a1b2c3d4` | **日付**: 2026-02-14 | **仕様書**: `specs/SPEC-a1b2c3d4/spec.md`

## 目的

- エージェント起動回数とワークツリー作成回数を `~/.gwt/stats.toml` に永続化する
- ステータスバーに CPU/メモリの ASCII バー + テキストをリアルタイム表示する
- About ダイアログを 3 タブ構成（General / System / Statistics）に拡張する

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-tauri/`, `crates/gwt-core/`）
- **フロントエンド**: Svelte 5 + TypeScript（`gwt-gui/`）
- **新規依存**:
  - `sysinfo` v0.32（`default-features = false, features = ["system"]`）→ CPU/メモリ
  - `nvml-wrapper` v0.10（オプション、Linux/Windows のみ）→ NVIDIA GPU
- **テスト**: cargo test / vitest + @testing-library/svelte
- **既存活用**: `toml` v0.9, `serde`, `dirs` v6, `tracing`（ワークスペース既存）
- **config パターン**: `write_atomic()` + `ensure_config_dir()` + `backup_broken_file()`（migration.rs）
- **AppState パターン**: `Mutex<T>` ラップ + `AppState::new()` 初期化

## 実装方針

### Phase 1: 統計データストレージ（gwt-core）

#### 1-1. stats モジュールの新設

`crates/gwt-core/src/config/stats.rs`（新規ファイル）:

- `AgentStats` 構造体: エージェント種別×モデル別の起動回数マップ
- `WorktreeStats` 構造体: ワークツリー作成回数
- `Stats` ルート構造体: グローバル統計 + リポジトリ別統計のマップ
- `load()` / `save()` 関数: `~/.gwt/stats.toml` の読み書き（既存 `agent_config.rs` パターン準拠）
- `load()`: ファイル存在チェック → TOML パース → エラー時は `backup_broken_file()` + デフォルト返却
- `save()`: `ensure_config_dir()` → `toml::to_string_pretty()` → `write_atomic()` (temp + rename)
- `increment_agent_launch()`: エージェント種別・モデル・リポジトリパスを受け取り、グローバルとリポ別をインクリメント
- `increment_worktree_created()`: リポジトリパスを受け取り、グローバルとリポ別をインクリメント
- 破損ファイルのハンドリング: パースエラー時はデフォルト値で初期化

#### 1-2. stats.toml のデータ構造

TOML 形式でフラットに保つ工夫として、リポジトリパスやエージェント名をテーブルキーにする:

```toml
[global.agents]
"claude-code.claude-sonnet-4-5-20250929" = 42
"claude-code.claude-opus-4-6" = 15
"codex.o3" = 8
"custom-agent.default" = 3

[global]
worktrees_created = 25

[repos."/Users/user/projects/my-app".agents]
"claude-code.claude-sonnet-4-5-20250929" = 20
"claude-code.claude-opus-4-6" = 5

[repos."/Users/user/projects/my-app"]
worktrees_created = 10
```

### Phase 2: 統計記録のフック（gwt-tauri）

#### 2-1. エージェント起動時の統計記録

`crates/gwt-tauri/src/commands/terminal.rs`:

- `launch_agent_for_project_root()` 内で、エージェント起動要求時に `increment_agent_launch()` を呼び出す
- 引数: agent_id（"claude-code" 等）、model（"claude-sonnet" 等）、repo_path
- 非同期で stats.toml に書き込み（起動のクリティカルパスをブロックしない）
- サブエージェント起動時も同様にカウント

#### 2-2. ワークツリー作成時の統計記録

`crates/gwt-tauri/src/commands/terminal.rs` の `resolve_worktree_path()` / `create_new_worktree_path()`:

- ワークツリーが新規作成された場合（created == true）に `increment_worktree_created()` を呼び出す

### Phase 3: システム情報取得（gwt-core / gwt-tauri）

#### 3-1. sysinfo によるシステム情報取得

`crates/gwt-core/src/system_info.rs`（新規ファイル）:

- `SystemMonitor` 構造体: sysinfo::System をラップ
- `new()`: System を初期化（CPU/メモリ情報を有効化）
- `refresh()`: CPU/メモリの最新値を取得
- `cpu_usage()`: CPU 使用率（%）を返す
- `memory_info()`: (used_bytes, total_bytes) を返す
- `gpu_info()`: GPU 静的情報（モデル名、VRAM）を返す

#### 3-2. NVIDIA GPU 情報取得

`crates/gwt-core/src/system_info.rs` 内:

- `#[cfg(feature = "nvidia-gpu")]` で条件コンパイル（macOS ではビルドしない）
- nvml-wrapper を初期化（`Nvml::init()` 失敗時は None で保持）
- `gpu_dynamic_info()`: GPU 使用率・VRAM 使用量を返す（NVIDIA のみ）
- Cargo.toml で `[target.'cfg(any(target_os = "linux", target_os = "windows"))'.dependencies]` を使用
- 非NVIDIA 環境ではグレースフルデグレード（gpu_static_info のみ）

#### 3-3. Tauri コマンド

`crates/gwt-tauri/src/commands/system.rs`（新規ファイル）:

- `get_system_info` コマンド: CPU 使用率・メモリ情報・GPU 情報を返す
- `get_stats` コマンド: stats.toml を読み込んで統計データを返す
- SystemMonitor を AppState に保持し、呼び出しごとに refresh

### Phase 4: ステータスバー拡張（gwt-gui）

#### 4-1. システム情報ポーリング

`gwt-gui/src/lib/systemMonitor.ts`（新規ファイル）:

- `createSystemMonitor()`: 1秒間隔で `get_system_info` を invoke するポーリング関数
- Svelte 5 の `$state` で CPU/メモリ値を保持
- `document.visibilitychange` イベントでポーリングの開始/停止を制御
- 初回は2回連続取得（sysinfo の初回 0% 問題対策）

#### 4-2. StatusBar.svelte の拡張

`gwt-gui/src/lib/components/StatusBar.svelte`:

- spacer の右側（path の左）にシステム情報セクションを追加
- ASCII バー生成ヘルパー: 値に応じて `|` 文字でバーを描画（8文字幅）
- 色分け: `colorClass(pct)` → 70%未満="ok"、70-90%="warn"、90%以上="bad"
- フォーマット: `CPU [||||    ] 45%  MEM [||||||  ] 8.2/16G`
- クリックイベントを dispatch してAboutを開く
- ステータスバー高さを 28px に変更

### Phase 5: About ダイアログ拡張（gwt-gui）

#### 5-1. About コンポーネントのリファクタリング

`gwt-gui/src/lib/components/AboutDialog.svelte`（新規コンポーネント）:

- App.svelte のインラインAboutを独立コンポーネントに抽出
- props: `open`, `initialTab`（"general" | "system" | "statistics"）
- タブ切り替え UI（General / System / Statistics）
- タブ選択状態の管理

#### 5-2. General タブ

- 既存の About 情報（アプリ名、バージョン、エディション）
- 検出済みエージェント一覧（detect_agents の結果を利用）

#### 5-3. System タブ

- CPU: 使用率 + ASCII バー（ステータスバーと同じ形式だが詳細版）
- メモリ: 使用量/総量 + パーセンテージ + ASCII バー
- GPU: モデル名、VRAM（NVIDIA: 使用率・VRAM使用量のリアルタイム）
- About が開いている間のみ詳細ポーリング（ステータスバーと共有可能）

#### 5-4. Statistics タブ

- エージェント起動回数テーブル: Agent | Model | Count
- ドロップダウン: "All repositories" (デフォルト) + リポジトリ一覧
- ワークツリー作成回数: グローバル累計表示
- stats.toml がない場合の空状態表示

### Phase 6: 統合とテスト

#### 6-1. App.svelte の修正

- インラインAboutモーダルを AboutDialog コンポーネントに置き換え
- メニューからの About → `initialTab: "general"`
- ステータスバーからの About → `initialTab: "system"`
- StatusBar に systemMonitor のデータを渡す

## リスク

| ID | リスク | 影響 | 軽減策 |
|---|---|---|---|
| RISK-001 | sysinfo のバイナリサイズ増加 | アプリサイズが肥大化 | `default-features = false, features = ["system"]` で最小化 |
| RISK-002 | nvml-wrapper が macOS でコンパイル不可 | CI ビルドエラー | `cfg(any(target_os = "linux", target_os = "windows"))` + optional feature |
| RISK-003 | stats.toml の同時書き込み | データ損失 | `write_atomic()` (temp + rename) + AppState 内 Mutex |
| RISK-004 | ステータスバー高さ拡張による layout shift | ターミナル領域縮小 | 24→28px の最小拡張（4px 増） |
| RISK-005 | sysinfo 初回 CPU 0% 問題 | 起動直後に誤表示 | フロントエンドで初回値を捨て、2回目から表示 |
| RISK-006 | TOML キーにリポジトリパスの特殊文字 | パースエラー | serde の TOML キー自動エスケープ（引用符囲み） |

## 依存関係

- Phase 1（統計ストレージ）→ Phase 2（統計記録フック）→ Phase 5-4（Statistics タブ）
- Phase 3（システム情報取得）→ Phase 4（ステータスバー）→ Phase 5-3（System タブ）
- Phase 5-1（About コンポーネント）→ Phase 5-2〜5-4（各タブ）→ Phase 6（統合）

## マイルストーン

| マイルストーン | 内容 | 完了条件 |
|---|---|---|
| M1: 統計記録 | stats.toml にエージェント起動回数・WT作成回数が記録される | SC-001, SC-002 |
| M2: ステータスバー | CPU/メモリが ASCII バーで表示される | SC-003 |
| M3: About 基盤 | 3タブ構成の About ダイアログが動作 | SC-005 |
| M4: System タブ | CPU/MEM/GPU リアルタイム表示 | SC-004, SC-006 |
| M5: Statistics タブ | 起動回数テーブル + フィルタ | SC-005 |
| M6: 堅牢性 | 破損ファイル対応・ポーリング制御 | SC-007 |

## テスト

### バックエンド

- stats.rs: increment/load/save のユニットテスト（正常・破損・空ファイル）
- system_info.rs: SystemMonitor の基本動作テスト
- アトミック書き込みのテスト（temp+rename）

### フロントエンド

- StatusBar: CPU/MEM 表示の色分けテスト
- AboutDialog: タブ切り替え・initialTab テスト
- Statistics タブ: テーブルレンダリング・フィルタテスト
- systemMonitor: ポーリング開始/停止テスト
