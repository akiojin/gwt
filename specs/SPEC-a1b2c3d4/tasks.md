# タスクリスト: エージェント起動統計・システムモニター・About強化

## ストーリー間依存関係

```text
Phase 1 (セットアップ) ─→ Phase 2 (基盤)
                              ├─→ Phase 3 (US1 起動記録 + US2 WT記録)
                              ├─→ Phase 4 (US3 ステータスバー)
                              └─→ Phase 5 (US5 About General)
                                    ├─→ Phase 6 (US4 ステータスバー→About + US6 System)
                                    └─→ Phase 7 (US7 Statistics)
```

- US1/US2: stats ストレージ基盤に依存（Phase 2）
- US3: SystemMonitor + Tauri コマンドに依存（Phase 2）
- US4: ステータスバー（US3）+ About コンポーネント（US5）に依存
- US5: About コンポーネント基盤。US4/US6/US7 の前提
- US6: About コンポーネント（US5）+ SystemMonitor に依存
- US7: About コンポーネント（US5）+ stats ストレージに依存

## Phase 1: セットアップ

- [x] T001 [P] `sysinfo` crate をワークスペース Cargo.toml の `[workspace.dependencies]` に追加する。`sysinfo = { version = "0.32", default-features = false, features = ["system"] }` `Cargo.toml`
- [x] T002 [P] `nvml-wrapper` crate をワークスペース Cargo.toml の `[workspace.dependencies]` に追加する。`nvml-wrapper = { version = "0.10", optional = true }` `Cargo.toml`
- [x] T003 [P] `crates/gwt-core/Cargo.toml` の `[dependencies]` に `sysinfo.workspace = true` を追加する `crates/gwt-core/Cargo.toml`
- [x] T004 [P] `crates/gwt-core/Cargo.toml` に `[features]` セクションで `nvidia-gpu = ["dep:nvml-wrapper"]` を追加し、`[target.'cfg(any(target_os = "linux", target_os = "windows"))'.dependencies]` に `nvml-wrapper = { workspace = true, optional = true }` を追加する `crates/gwt-core/Cargo.toml`
- [x] T005 [P] `crates/gwt-core/src/config/mod.rs` に `pub mod stats;` を追加する `crates/gwt-core/src/config/mod.rs`
- [x] T006 [P] `crates/gwt-core/src/lib.rs` に `pub mod system_info;` を追加する `crates/gwt-core/src/lib.rs`
- [x] T007 [P] コンパイル用の空ファイル `crates/gwt-core/src/config/stats.rs` を作成する `crates/gwt-core/src/config/stats.rs`
- [x] T008 [P] コンパイル用の空ファイル `crates/gwt-core/src/system_info.rs` を作成する `crates/gwt-core/src/system_info.rs`
- [x] T009 [P] コンパイル用の空ファイル `crates/gwt-tauri/src/commands/system.rs` を作成し、`crates/gwt-tauri/src/commands/mod.rs` に `pub mod system;` を追加する `crates/gwt-tauri/src/commands/system.rs` `crates/gwt-tauri/src/commands/mod.rs`
- [x] T010 `cargo check` でワークスペース全体がコンパイルできることを確認する

## Phase 2: 基盤

### 2A: 統計ストレージ（US1/US2 共通）

- [x] T011 [US1,US2] stats モジュールのユニットテストを作成する（TDD: RED）。テストケース: (1) 空の `Stats` が `Stats::default()` で生成できる (2) `Stats::default()` を TOML にシリアライズし、再度デシリアライズするラウンドトリップが成功する `crates/gwt-core/src/config/stats.rs`
- [x] T012 [US1] `increment_agent_launch` のユニットテストを作成する（TDD: RED）。テストケース: (1) 空の Stats に対して `increment_agent_launch("claude-code", "claude-sonnet", "/path/repo")` を呼ぶとグローバル agents の `"claude-code.claude-sonnet"` が 1 になる (2) 同じ呼び出しを2回行うと 2 になる (3) 別のリポジトリで呼ぶとグローバルは 2、新リポジトリは 1 になる `crates/gwt-core/src/config/stats.rs`
- [x] T013 [US2] `increment_worktree_created` のユニットテストを作成する（TDD: RED）。テストケース: (1) 空の Stats に対して `increment_worktree_created("/path/repo")` を呼ぶとグローバル worktrees_created が 1 になる (2) 別リポジトリで呼ぶとグローバルは 2、各リポジトリは 1 `crates/gwt-core/src/config/stats.rs`
- [x] T014 [US1,US2] `Stats` / `StatsEntry` 構造体を定義する（TDD: GREEN for T011）。`Stats { global: StatsEntry, repos: HashMap<String, StatsEntry> }`、`StatsEntry { agents: HashMap<String, u64>, worktrees_created: u64 }`。`#[derive(Debug, Clone, Default, Serialize, Deserialize)]` を付与する `crates/gwt-core/src/config/stats.rs`
- [x] T015 [US1] `Stats::increment_agent_launch(&mut self, agent_id: &str, model: &str, repo_path: &str)` を実装する（TDD: GREEN for T012）。キーは `"{agent_id}.{model}"`（model が空なら `"default"`）。global.agents と repos[repo_path].agents をそれぞれ +1 する `crates/gwt-core/src/config/stats.rs`
- [x] T016 [US2] `Stats::increment_worktree_created(&mut self, repo_path: &str)` を実装する（TDD: GREEN for T013）。global.worktrees_created と repos[repo_path].worktrees_created をそれぞれ +1 する `crates/gwt-core/src/config/stats.rs`
- [x] T017 [US1,US2] `load` / `save` のユニットテストを作成する（TDD: RED）。テストケース: (1) 存在しないパスで `Stats::load()` するとデフォルト Stats が返る (2) `save()` → `load()` のラウンドトリップでデータが一致する (3) 不正な TOML ファイルに対して `load()` するとデフォルト Stats が返る（破損ファイルは `.broken` にリネームされる） `crates/gwt-core/src/config/stats.rs`
- [x] T018 [US1,US2] `Stats::toml_path()` を実装する。`dirs::home_dir().unwrap_or(".").join(".gwt").join("stats.toml")` を返す `crates/gwt-core/src/config/stats.rs`
- [x] T019 [US1,US2] `Stats::load()` を実装する（TDD: GREEN for T017）。ファイル存在チェック → `toml::from_str()` → エラー時は `backup_broken_file()` + `Stats::default()` 返却。`tracing::warn!` でログ出力 `crates/gwt-core/src/config/stats.rs`
- [x] T020 [US1,US2] `Stats::save(&self)` を実装する（TDD: GREEN for T017）。`ensure_config_dir()` → `toml::to_string_pretty()` → `write_atomic()` (temp + rename)。`tracing::info!` でログ出力 `crates/gwt-core/src/config/stats.rs`
- [x] T021 [US1,US2] `cargo test -p gwt-core` で stats モジュールの全テストが GREEN であることを確認する

### 2B: システム情報取得（US3/US6 共通）

- [x] T022 [US3,US6] `SystemMonitor` のユニットテストを作成する（TDD: RED）。テストケース: (1) `SystemMonitor::new()` が正常に初期化できる (2) `refresh()` 後に `cpu_usage()` が 0.0〜100.0 の範囲を返す (3) `memory_info()` の total が 0 より大きい `crates/gwt-core/src/system_info.rs`
- [x] T023 [US3,US6] `SystemMonitor` 構造体を定義する。内部に `sysinfo::System` を保持する。`new()` で `System::new_with_specifics(RefreshKind::new().with_cpu(CpuRefreshKind::everything()).with_memory(MemoryRefreshKind::everything()))` で初期化する `crates/gwt-core/src/system_info.rs`
- [x] T024 [US3,US6] `SystemMonitor::refresh(&mut self)` を実装する。`self.sys.refresh_cpu_usage()` と `self.sys.refresh_memory()` を呼び出す `crates/gwt-core/src/system_info.rs`
- [x] T025 [US3,US6] `SystemMonitor::cpu_usage(&self) -> f32` を実装する。`self.sys.global_cpu_usage()` を返す `crates/gwt-core/src/system_info.rs`
- [x] T026 [US3,US6] `SystemMonitor::memory_info(&self) -> (u64, u64)` を実装する。`(self.sys.used_memory(), self.sys.total_memory())` を返す `crates/gwt-core/src/system_info.rs`
- [x] T027 [US6] `SystemMonitor::gpu_static_info(&self) -> Option<GpuStaticInfo>` を実装する。sysinfo の `Components` feature 未有効のため常に None を返す。NVIDIA 環境では `gpu_dynamic_info()` で補完 `crates/gwt-core/src/system_info.rs`
- [x] T028 [US6] NVIDIA GPU 動的情報取得を実装する。`#[cfg(feature = "nvidia-gpu")]` ブロック内で `Nvml::init()` を試行し、成功時は `gpu_dynamic_info()` で使用率・VRAM 使用量を返す。失敗時は None。`#[cfg(not(feature = "nvidia-gpu"))]` では常に None を返すスタブを定義する `crates/gwt-core/src/system_info.rs`
- [x] T029 [US3,US6] `cargo test -p gwt-core` で system_info モジュールの全テストが GREEN であることを確認する

### 2C: Tauri コマンド（US3/US6/US7 共通）

- [x] T030 [US3,US6] `SystemInfoResponse` / `GpuInfo` 構造体を定義する（`#[derive(Serialize)]`）。`contracts/tauri-commands.md` の `get_system_info` レスポンス形式に準拠する `crates/gwt-tauri/src/commands/system.rs`
- [x] T031 [US7] `StatsResponse` / `StatsEntryResponse` / `AgentStatEntry` / `RepoStatsEntry` 構造体を定義する（`#[derive(Serialize)]`）。`contracts/tauri-commands.md` の `get_stats` レスポンス形式に準拠する `crates/gwt-tauri/src/commands/system.rs`
- [x] T032 [US3,US6] `AppState` に `pub system_monitor: Mutex<SystemMonitor>` フィールドを追加し、`AppState::new()` で `Mutex::new(SystemMonitor::new())` に初期化する `crates/gwt-tauri/src/state.rs`
- [x] T033 [US3,US6] Tauri コマンド `get_system_info` を実装する。`AppState.system_monitor` を lock → `refresh()` → `cpu_usage()` / `memory_info()` / `gpu_static_info()` / `gpu_dynamic_info()` を取得し `SystemInfoResponse` に変換して返す `crates/gwt-tauri/src/commands/system.rs`
- [x] T034 [US7] Tauri コマンド `get_stats` を実装する。`Stats::load()` を呼び出し、`HashMap<String, u64>` のエージェントマップを `Vec<AgentStatEntry>` に変換（キーを `"."` で agent_id と model に分割）して `StatsResponse` を返す `crates/gwt-tauri/src/commands/system.rs`
- [x] T035 `get_system_info` と `get_stats` を `generate_handler!` マクロに登録する `crates/gwt-tauri/src/app.rs`
- [x] T036 `cargo check` でコンパイルが通ることを確認する

## Phase 3: US1 エージェント起動回数の記録 + US2 ワークツリー作成回数の記録 (P0)

- [x] T037 [US1] `launch_agent_for_project_root()` 関数内で、ワークツリー作成判定後に統計記録ロジックを追加。`std::thread::spawn` で非同期に `Stats::load()` → `increment_agent_launch(agent_id, model, repo_path)` → `Stats::save()` を実行。save エラーはログ出力のみで起動をブロックしない `crates/gwt-tauri/src/commands/terminal.rs`
- [x] T038 [US1] サブエージェント起動も同一の `launch_agent_for_project_root()` を経由するため、T037 で全パスをカバー済み `crates/gwt-tauri/src/commands/terminal.rs`
- [x] T039 [US2] `launch_agent_for_project_root()` 内の統計記録フックで `worktree_created` フラグに基づき `increment_worktree_created()` を呼び出す。`resolve_worktree_path()` が `created==true` を返すケースをカバー `crates/gwt-tauri/src/commands/terminal.rs`
- [x] T040 [US2] `create_new_worktree_path()` 経由のワークツリー作成も `launch_agent_for_project_root()` 内で `worktree_created=true` として統計記録される `crates/gwt-tauri/src/commands/terminal.rs`
- [x] T041 [US1,US2] `cargo check` でコンパイルが通ることを確認する

## Phase 4: US3 ステータスバーに CPU/メモリ表示 (P0)

- [x] T042 [US3] `gwt-gui/src/lib/systemMonitor.ts`（新規）を作成する。`createSystemMonitor()` 関数を定義: `$state` で `cpuUsage: number = 0`, `memUsed: number = 0`, `memTotal: number = 0`, `gpuInfo: GpuInfo | null = null` を保持。1秒間隔の `setInterval` で `invoke("get_system_info")` を呼び出し state を更新する。`start()` / `stop()` / `destroy()` メソッドを提供する `gwt-gui/src/lib/systemMonitor.ts`
- [x] T043 [US3] `systemMonitor.ts` に `document.visibilitychange` リスナーを追加する。`document.hidden === true` で `stop()`、`false` で `start()` を呼び出す `gwt-gui/src/lib/systemMonitor.ts`
- [x] T044 [US3] `systemMonitor.ts` の初期化時に `invoke("get_system_info")` を2回連続呼び出す（sysinfo 初回 0% 問題の回避）。最初の結果は捨て、2回目から表示に使用する `gwt-gui/src/lib/systemMonitor.ts`
- [x] T045 [US3] StatusBar.svelte に新しい props を追加する: `cpuUsage: number`, `memUsed: number`, `memTotal: number`。型定義を更新する `gwt-gui/src/lib/components/StatusBar.svelte`
- [x] T046 [US3] StatusBar.svelte に ASCII バー生成ヘルパー関数 `renderBar(pct: number): string` を追加する。8文字幅で `|` を `Math.round(pct / 100 * 8)` 個描画し、残りはスペースで埋める。結果を `[||||    ]` 形式で返す `gwt-gui/src/lib/components/statusBarHelpers.ts`
- [x] T047 [US3] StatusBar.svelte に色分けヘルパー関数 `usageColorClass(pct: number): string` を追加する。70%未満="ok"、70以上90未満="warn"、90以上="bad" を返す `gwt-gui/src/lib/components/statusBarHelpers.ts`
- [x] T048 [US3] StatusBar.svelte に `formatMemory(bytes: number): string` ヘルパーを追加する。GB 単位で小数1桁にフォーマットする（例: `8589934592` → `"8.0"`） `gwt-gui/src/lib/components/statusBarHelpers.ts`
- [x] T049 [US3] StatusBar.svelte の `<span class="spacer"></span>` の直後（`<span class="status-item path">` の直前）にシステム情報セクションを追加する。`<span class="status-item system-info">` 内に CPU と MEM の ASCII バー + パーセンテージをレンダリングする。フォーマット例: `CPU [||||    ] 45%  MEM [||||||  ] 8.0/16.0G` `gwt-gui/src/lib/components/StatusBar.svelte`
- [x] T050 [US3] StatusBar.svelte の `.system-info` 内の各要素に `usageColorClass()` のクラスを適用する CSS を追加する。`.system-info .ok { color: var(--green); }` `.system-info .warn { color: var(--yellow); }` `.system-info .bad { color: var(--red); }` `gwt-gui/src/lib/components/StatusBar.svelte`
- [x] T051 [US3] `gwt-gui/src/styles/global.css` の `--statusbar-height` を `24px` から `28px` に変更する `gwt-gui/src/styles/global.css`
- [x] T052 [US3] App.svelte で `systemMonitor` を初期化する。`onMount` で `createSystemMonitor()` を呼び出し、`start()` する。`onDestroy` で `destroy()` する。monitor の state を StatusBar コンポーネントの props に渡す `gwt-gui/src/App.svelte`

## Phase 5: US5 About General タブ + About コンポーネント基盤 (P0)

- [x] T053 [US5] `gwt-gui/src/lib/components/AboutDialog.svelte`（新規）を作成する。props: `open: boolean`, `initialTab: "general" | "system" | "statistics" = "general"`, `onclose: () => void`。3つのタブボタン（General / System / Statistics）と `activeTab` state を持つ。`open` が変化したとき `activeTab` を `initialTab` にリセットする。overlay + dialog のモーダル構造。Close ボタン `gwt-gui/src/lib/components/AboutDialog.svelte`
- [x] T054 [US5] AboutDialog.svelte の General タブ内容を実装する。アプリ名「gwt」、「Git Worktree Manager」、「GUI Edition」、バージョン表示（`getAppVersionSafe()` を利用）。下部に検出済みエージェント一覧を表示する（`invoke("detect_agents")` を呼び出し、エージェント名・バージョンのリスト） `gwt-gui/src/lib/components/AboutDialog.svelte`
- [x] T055 [US5] AboutDialog.svelte にタブ切り替え・モーダル表示・Close ボタンの CSS スタイルを追加する。既存の About モーダルのスタイルを参考にしつつ、タブ UI を追加する `gwt-gui/src/lib/components/AboutDialog.svelte`
- [x] T056 [US5] App.svelte のインライン About モーダル（`{#if showAbout}` ブロック全体と対応する CSS `.about-dialog` 等）を削除し、`<AboutDialog open={showAbout} initialTab={aboutInitialTab} onclose={() => showAbout = false} />` に置き換える。`aboutInitialTab` state を新規追加する `gwt-gui/src/App.svelte`
- [x] T057 [US5] App.svelte の menu-action ハンドラの `"about"` ケースで `aboutInitialTab = "general"; showAbout = true;` を設定する `gwt-gui/src/App.svelte`

## Phase 6: US4 ステータスバーから About を開く + US6 About System タブ (P0)

- [x] T058 [US4] StatusBar.svelte のシステム情報セクション（`.system-info`）にクリックイベントを追加する。クリック時に props の `onopenAboutSystem` コールバックを呼び出す `gwt-gui/src/lib/components/StatusBar.svelte`
- [x] T059 [US4] StatusBar.svelte の props に `onopenAboutSystem?: () => void` を追加する `gwt-gui/src/lib/components/StatusBar.svelte`
- [x] T060 [US4] App.svelte の StatusBar に `onopenAboutSystem` ハンドラを渡す。ハンドラ内で `aboutInitialTab = "system"; showAbout = true;` を設定する。`showAbout` が既に `true` の場合は何もしない（二重表示防止） `gwt-gui/src/App.svelte`
- [x] T061 [US6] AboutDialog.svelte の System タブ内容を実装する。systemMonitor の CPU/メモリデータを props で受け取り、CPU 使用率 + ASCII バー + パーセンテージ、メモリ使用量/総量 + ASCII バー + パーセンテージを表示する。ステータスバーより詳細なレイアウト（ラベル + バー + 数値を3行で表示） `gwt-gui/src/lib/components/AboutDialog.svelte`
- [x] T062 [US6] AboutDialog.svelte の System タブに GPU 情報セクションを追加する。`gpu` が非 null の場合にモデル名を表示。NVIDIA 環境（`usage_percent` が非 null）の場合は使用率 + VRAM 使用量/総量も表示。GPU 未検出の場合は「No GPU detected」表示 `gwt-gui/src/lib/components/AboutDialog.svelte`
- [x] T063 [US6] AboutDialog.svelte の props に systemMonitor データを追加する: `cpuUsage: number`, `memUsed: number`, `memTotal: number`, `gpuInfo: GpuInfo | null` `gwt-gui/src/lib/components/AboutDialog.svelte`
- [x] T064 [US6] App.svelte の AboutDialog に systemMonitor のデータを渡す props を追加する `gwt-gui/src/App.svelte`

## Phase 7: US7 About Statistics タブ (P0)

- [x] T065 [US7] AboutDialog.svelte の Statistics タブ内容を実装する。タブが表示された時に `invoke("get_stats")` でデータを取得する。エージェント起動回数テーブル（Agent | Model | Count の HTML table）を表示する。データなしの場合は「No statistics yet」メッセージ `gwt-gui/src/lib/components/AboutDialog.svelte`
- [x] T066 [US7] Statistics タブにリポジトリフィルタドロップダウンを追加する。選択肢: "All repositories"（デフォルト）+ `stats.repos` のキー一覧。選択変更時にテーブルの表示データを切り替える（"All repositories" = global、特定リポジトリ = repos[path]） `gwt-gui/src/lib/components/AboutDialog.svelte`
- [x] T067 [US7] Statistics タブにワークツリー作成回数セクションを追加する。選択されたスコープ（グローバルまたはリポジトリ別）の `worktrees_created` を表示する `gwt-gui/src/lib/components/AboutDialog.svelte`

## Phase 8: フロントエンドテスト

- [x] T068 [P] [US3] StatusBar のシステム情報表示テストを作成する。テストケース: (1) cpuUsage=50 のとき `renderBar(50)` が `[||||    ]` を返す (2) cpuUsage=75 のとき `usageColorClass(75)` が `"warn"` を返す (3) cpuUsage=95 のとき `usageColorClass(95)` が `"bad"` を返す (4) `formatMemory(8589934592)` が `"8.0"` を返す `gwt-gui/src/lib/components/__tests__/StatusBar.test.ts`
- [x] T069 [P] [US5,US6,US7] AboutDialog のテストを作成する。テストケース: (1) `initialTab="general"` で General タブがアクティブ表示される (2) `initialTab="system"` で System タブがアクティブ表示される (3) タブボタンクリックでタブが切り替わる (4) Close ボタンクリックで `onclose` が呼ばれる `gwt-gui/src/lib/components/__tests__/AboutDialog.test.ts`

## Phase 9: 仕上げ・横断

- [x] T070 [P] [共通] `specs/specs.md` の現行仕様テーブルに SPEC-a1b2c3d4 が登録されていることを確認する（既に登録済みならスキップ） `specs/specs.md`
- [x] T071 [共通] `cargo clippy --all-targets --all-features -- -D warnings` でバックエンド全体の lint を通す
- [x] T072 [共通] `cargo fmt --check` でフォーマットを検証する。差分があれば `cargo fmt` で修正する
- [x] T073 [共通] `cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json` でフロントエンドの型チェックを通す
- [x] T074 [共通] `cargo test` で全バックエンドテストを通す
- [x] T075 [共通] `cd gwt-gui && npx vitest run` で全フロントエンドテストを通す（WorktreeSummaryPanel の既存失敗1件を除く）
