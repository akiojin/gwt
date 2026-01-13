# 実装計画: TypeScript/Bun から Rust への完全移行

## 概要

5つのフェーズに分けて段階的に移行を実施する。
各フェーズは独立してテスト可能な単位とし、品質を確保しながら進める。

## 技術スタック確定

### 依存クレート

**Cargo.toml (workspace)**:

```toml
[workspace]
resolver = "2"
members = ["crates/*"]

[workspace.package]
version = "5.0.0"
edition = "2021"
license = "MIT"
repository = "https://github.com/akiojin/gwt"

[workspace.dependencies]
# Async
tokio = { version = "1", features = ["full"] }

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"

# Error handling (thiserrorのみ、anyhowは使用しない)
thiserror = "2"

# Logging (JSON Lines + スパン)
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["json", "env-filter"] }

# Git (gix + 外部gitフォールバック)
gix = { version = "0.68", features = ["blocking-network-client"] }

# CLI
clap = { version = "4", features = ["derive"] }

# TUI (ratatui-async-template ベース)
ratatui = "0.29"
crossterm = "0.28"

# Web
axum = "0.7"
tower = "0.4"
tower-http = { version = "0.6", features = ["fs", "cors"] }

# Config (TOML形式、マイグレーション対応)
figment = { version = "0.10", features = ["toml", "json", "env"] }
dirs = "5"

# File locking (マルチインスタンス対応)
fs2 = "0.4"

# Utilities
uuid = { version = "1", features = ["v4"] }
chrono = { version = "0.4", features = ["serde"] }

# Benchmark
criterion = { version = "0.5", features = ["html_reports"] }
```

## Phase 1: 基盤構築

### 1.1 プロジェクト初期化

```bash
cargo new gwt-rust --name gwt
cd gwt-rust
mkdir -p crates/{gwt-core,gwt-cli,gwt-web,gwt-frontend}
mkdir -p benches tests/{integration,e2e} messages
```

### 1.2 エラー型設計 (thiserror + カテゴリ別コード)

**ファイル**: `crates/gwt-core/src/error/mod.rs`

```rust
use thiserror::Error;

/// エラーコード体系
/// E1xxx: Git操作
/// E2xxx: Worktree
/// E3xxx: 設定
/// E4xxx: ログ
/// E5xxx: Agent
/// E6xxx: Web
/// E9xxx: 一般

#[derive(Debug, Error)]
pub enum GitError {
    #[error("[E1001] Branch not found: {name}")]
    BranchNotFound { name: String },

    #[error("[E1002] Repository not found")]
    RepositoryNotFound,

    #[error("[E1003] Git command not installed")]
    GitNotInstalled,

    #[error("[E1004] Uncommitted changes exist")]
    UncommittedChanges,

    #[error("[E1005] Unpushed commits exist")]
    UnpushedCommits,

    #[error("[E1006] Fetch failed: {reason}")]
    FetchFailed { reason: String },

    #[error("[E1007] Fast-forward pull failed: {reason}")]
    FastForwardFailed { reason: String },

    // ... 他のGitエラー
}

#[derive(Debug, Error)]
pub enum WorktreeError {
    #[error("[E2001] Failed to create worktree: {reason}")]
    CreateFailed { reason: String },

    #[error("[E2002] Failed to remove worktree: {reason}")]
    RemoveFailed { reason: String },

    #[error("[E2003] Worktree already exists: {path}")]
    AlreadyExists { path: String },

    #[error("[E2004] Worktree locked by another process")]
    Locked,

    #[error("[E2005] Orphaned worktree detected: {path}")]
    Orphaned { path: String },

    // ... 他のWorktreeエラー
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("[E3001] Failed to parse config: {reason}")]
    ParseError { reason: String },

    #[error("[E3002] Migration failed: {reason}")]
    MigrationFailed { reason: String },

    // ... 他の設定エラー
}
```

**エラーメッセージファイル**: `messages/errors.toml`

```toml
[git]
E1001 = "Branch not found"
E1002 = "Repository not found"
E1003 = "Git command is required but not installed"

[worktree]
E2001 = "Failed to create worktree"
E2002 = "Failed to remove worktree"
```

### 1.3 Git操作モジュール (gix + フォールバック)

**ファイル**: `crates/gwt-core/src/git/mod.rs`

```rust
use std::process::Command;

pub struct Repository {
    gix_repo: Option<gix::Repository>,
    path: PathBuf,
}

impl Repository {
    /// 起動時にgitコマンドの存在をチェック
    pub fn check_git_installed() -> Result<(), GitError> {
        Command::new("git")
            .arg("--version")
            .output()
            .map_err(|_| GitError::GitNotInstalled)?;
        Ok(())
    }

    /// リポジトリ検出
    pub fn discover() -> Result<Self, GitError> {
        let gix_repo = gix::discover(".")
            .map_err(|_| GitError::RepositoryNotFound)?;
        Ok(Self {
            path: gix_repo.path().to_path_buf(),
            gix_repo: Some(gix_repo),
        })
    }

    /// Worktree追加 (gix未実装のため外部gitコマンド使用)
    pub fn add_worktree(&self, branch: &str, path: &Path) -> Result<(), GitError> {
        let output = Command::new("git")
            .args(["worktree", "add", path.to_str().unwrap(), branch])
            .current_dir(&self.path)
            .output()
            .map_err(|e| GitError::CommandFailed { reason: e.to_string() })?;

        if !output.status.success() {
            return Err(GitError::WorktreeAddFailed {
                reason: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }
        Ok(())
    }
}
```

### 1.4 設定管理 (TOML + マイグレーション)

**ファイル**: `crates/gwt-core/src/config/mod.rs`

```rust
use figment::{Figment, providers::{Toml, Json, Env}};
use std::path::PathBuf;

pub struct ConfigManager {
    config_path: PathBuf,
}

impl ConfigManager {
    /// 設定ファイル探索順序（TypeScript版と同一）
    /// 1. .gwt.toml (新形式)
    /// 2. .gwt.json (旧形式 → 自動変換)
    /// 3. ~/.config/gwt/config.toml
    /// 4. ~/.config/gwt/config.json (旧形式 → 自動変換)
    pub fn load() -> Result<Settings, ConfigError> {
        // JSON → TOML 自動マイグレーション
        Self::migrate_json_to_toml()?;

        let figment = Figment::new()
            .merge(Toml::file(".gwt.toml"))
            .merge(Toml::file(dirs::config_dir().unwrap().join("gwt/config.toml")))
            .merge(Env::prefixed("GWT_"));

        figment.extract().map_err(|e| ConfigError::ParseError {
            reason: e.to_string(),
        })
    }

    /// JSON → TOML 自動変換
    fn migrate_json_to_toml() -> Result<(), ConfigError> {
        let json_path = PathBuf::from(".gwt.json");
        let toml_path = PathBuf::from(".gwt.toml");

        if json_path.exists() && !toml_path.exists() {
            let json_content = std::fs::read_to_string(&json_path)?;
            let value: serde_json::Value = serde_json::from_str(&json_content)?;
            let toml_content = toml::to_string_pretty(&value)?;
            std::fs::write(&toml_path, toml_content)?;
            // 元のJSONは削除しない（バックアップとして残す）
        }
        Ok(())
    }
}
```

### 1.5 ログシステム (JSON Lines + スパン)

**ファイル**: `crates/gwt-core/src/logging/mod.rs`

```rust
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use std::path::PathBuf;

pub fn init_logging(debug: bool) -> Result<(), LogError> {
    let log_dir = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("gwt/logs");
    std::fs::create_dir_all(&log_dir)?;

    let file_appender = tracing_appender::rolling::daily(&log_dir, "gwt.jsonl");

    let filter = if debug {
        EnvFilter::new("debug")
    } else {
        EnvFilter::from_default_env()
            .add_directive("gwt=info".parse().unwrap())
    };

    tracing_subscriber::registry()
        .with(filter)
        .with(
            fmt::layer()
                .json()
                .with_span_events(fmt::format::FmtSpan::CLOSE)
                .with_file(true)
                .with_line_number(true)
                .with_writer(file_appender)
        )
        .init();

    Ok(())
}
```

### 1.6 ファイルロック (マルチインスタンス対応)

**ファイル**: `crates/gwt-core/src/worktree/lock.rs`

```rust
use fs2::FileExt;
use std::fs::File;
use std::path::Path;

pub struct WorktreeLock {
    file: File,
}

impl WorktreeLock {
    /// Worktree単位でロックを取得
    pub fn acquire(worktree_path: &Path) -> Result<Self, WorktreeError> {
        let lock_path = worktree_path.join(".gwt.lock");
        let file = File::create(&lock_path)?;

        file.try_lock_exclusive()
            .map_err(|_| WorktreeError::Locked)?;

        Ok(Self { file })
    }
}

impl Drop for WorktreeLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}
```

### 1.7 基本CLI (clap + git存在チェック)

**ファイル**: `crates/gwt-cli/src/main.rs`

```rust
use clap::Parser;
use gwt_core::{git::Repository, logging};

#[derive(Parser)]
#[command(name = "gwt")]
#[command(about = "Git Worktree Manager")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Enable debug mode
    #[arg(long)]
    debug: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Start web UI server
    Serve {
        #[arg(short, long, default_value = "3001")]
        port: u16,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // ログ初期化
    logging::init_logging(cli.debug || std::env::var("GWT_DEBUG").is_ok())?;

    // git存在チェック（必須要件）
    Repository::check_git_installed()?;

    match cli.command {
        Some(Commands::Serve { port }) => serve(port).await,
        None => run_interactive().await,
    }
}
```

## Phase 2: CLI TUI

### 2.1 Elmアーキテクチャ実装

**ファイル**: `crates/gwt-cli/src/app/mod.rs`

```rust
use tokio::sync::mpsc;

/// Model: アプリケーション状態
pub struct Model {
    pub screen_stack: Vec<Screen>,
    pub branches: Vec<Branch>,
    pub current_screen: Screen,
    pub is_offline: bool,
    pub ctrl_c_count: u8,
}

/// Message: イベント
pub enum Message {
    KeyPress(KeyEvent),
    Tick,
    BranchesLoaded(Vec<Branch>),
    NetworkStatusChanged(bool),
    CtrlC,
}

/// Update: 状態更新
pub fn update(model: &mut Model, msg: Message) -> Option<Command> {
    match msg {
        Message::CtrlC => {
            model.ctrl_c_count += 1;
            if model.ctrl_c_count >= 2 {
                return Some(Command::Quit);
            }
            None
        }
        Message::KeyPress(key) => {
            model.ctrl_c_count = 0; // リセット
            handle_key(model, key)
        }
        // ...
    }
}

/// View: 描画
pub fn view(model: &Model, frame: &mut Frame) {
    // ヘッダー（オフライン表示含む）
    render_header(frame, model);

    // 現在の画面
    match &model.current_screen {
        Screen::BranchList(state) => render_branch_list(frame, state),
        Screen::WorktreeCreate(state) => render_worktree_create(frame, state),
        // ...
    }

    // フッター
    render_footer(frame, model);
}
```

### 2.2 画面スタック (状態保持)

```rust
pub struct ScreenStack {
    screens: Vec<(Screen, ScreenState)>,
}

impl ScreenStack {
    /// 新しい画面をプッシュ（前画面の状態を保持）
    pub fn push(&mut self, screen: Screen, state: ScreenState) {
        self.screens.push((screen, state));
    }

    /// 前の画面に戻る（状態復元）
    pub fn pop(&mut self) -> Option<(Screen, ScreenState)> {
        self.screens.pop()
    }

    /// 戻る時にスクロール位置等を復元
    pub fn restore_state(&self) -> Option<&ScreenState> {
        self.screens.last().map(|(_, state)| state)
    }
}
```

### 2.3 Ctrl+C二度押し終了

```rust
pub async fn run_event_loop(mut model: Model) -> Result<()> {
    let mut terminal = setup_terminal()?;
    let (tx, mut rx) = mpsc::channel(100);

    // Ctrl+Cハンドラ
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        loop {
            tokio::signal::ctrl_c().await.unwrap();
            tx_clone.send(Message::CtrlC).await.unwrap();
        }
    });

    loop {
        terminal.draw(|f| view(&model, f))?;

        tokio::select! {
            Some(msg) = rx.recv() => {
                if let Some(Command::Quit) = update(&mut model, msg) {
                    // クリーンアップ処理
                    cleanup(&model).await?;
                    break;
                }
            }
            // ... キーボードイベント等
        }
    }

    restore_terminal()?;
    Ok(())
}
```

### 2.4 遅延読み込み (1000+ブランチ対応)

```rust
pub struct BranchListState {
    branches: Vec<Branch>,
    loaded_count: usize,
    total_count: usize,
    scroll_offset: usize,
    selected: usize,
}

impl BranchListState {
    const PAGE_SIZE: usize = 50;

    /// スクロールに応じて追加ロード
    pub async fn load_more_if_needed(&mut self, repo: &Repository) {
        let visible_end = self.scroll_offset + Self::PAGE_SIZE;
        if visible_end > self.loaded_count && self.loaded_count < self.total_count {
            let more = repo.list_branches_range(self.loaded_count, Self::PAGE_SIZE).await?;
            self.branches.extend(more);
            self.loaded_count = self.branches.len();
        }
    }
}
```

## Phase 3: Coding Agent統合

### 3.1 Agent起動 (ブロッキング待機)

```rust
use std::process::{Command, Stdio};

pub struct ClaudeCode {
    mode: ClaudeMode,
}

impl ClaudeCode {
    pub async fn launch(&self, config: &AgentConfig) -> Result<i32, AgentError> {
        let mut cmd = Command::new("npx");
        cmd.arg("@anthropic-ai/claude-code");

        match &self.mode {
            ClaudeMode::Continue => { cmd.arg("--continue"); }
            ClaudeMode::Resume { session_id } => {
                cmd.arg("--resume").arg(session_id);
            }
            _ => {}
        }

        let mut child = cmd
            .envs(&config.env_vars)
            .current_dir(&config.working_dir)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| AgentError::LaunchFailed { reason: e.to_string() })?;

        // ブロッキング待機
        let status = child.wait()
            .map_err(|e| AgentError::WaitFailed { reason: e.to_string() })?;

        Ok(status.code().unwrap_or(-1))
    }
}
```

## Phase 4: Web UI

### 4.1 Axum サーバー (WASM埋め込み)

```rust
use axum::{Router, routing::get};
use tower_http::services::ServeDir;

// WASM/JS/CSSをバイナリに埋め込み
static WASM: &[u8] = include_bytes!("../../../gwt-frontend/pkg/gwt_frontend_bg.wasm");
static JS: &str = include_str!("../../../gwt-frontend/pkg/gwt_frontend.js");
static CSS: &str = include_str!("../../../gwt-frontend/pkg/style.css");

pub fn create_router() -> Router {
    Router::new()
        .route("/gwt_frontend_bg.wasm", get(|| async { WASM }))
        .route("/gwt_frontend.js", get(|| async { JS }))
        .route("/style.css", get(|| async { CSS }))
        .nest("/api", api_routes())
        .fallback(get(index_html))
}

async fn index_html() -> Html<&'static str> {
    Html(include_str!("../../../gwt-frontend/pkg/index.html"))
}
```

### 4.2 Leptos フロントエンド (CSRのみ)

```rust
// crates/gwt-frontend/src/lib.rs
use leptos::*;

#[component]
pub fn App() -> impl IntoView {
    view! {
        <Router>
            <Routes>
                <Route path="/" view=WorktreeList />
                <Route path="/branches" view=BranchList />
                <Route path="/terminal/:id" view=Terminal />
                <Route path="/settings" view=Settings />
            </Routes>
        </Router>
    }
}

#[component]
fn WorktreeList() -> impl IntoView {
    let worktrees = create_resource(|| (), |_| async {
        fetch_worktrees().await
    });

    view! {
        <div class="worktree-list">
            <Suspense fallback=|| view! { <p>"Loading..."</p> }>
                {move || worktrees.get().map(|wts| {
                    wts.into_iter().map(|wt| view! {
                        <WorktreeCard worktree=wt />
                    }).collect_view()
                })}
            </Suspense>
        </div>
    }
}
```

### 4.3 WebSocket/PTY/端末連携

- WebSocketはlocalhost限定で、接続時にPTYセッションを生成する。
- 入力はPTYへ転送し、出力はWebSocketへストリーミングする。
- 端末サイズ変更は専用メッセージで通知し、PTYサイズを更新する。
- 切断時は子プロセスとPTYを確実に破棄し、セッションを残さない。
- フロントエンドはxterm.jsで端末表示・入力・リサイズを扱う。
- wasm-opt最適化はTrunk設定で `z` を適用する。

## Phase 5: 品質・配布

### 5.1 統合テスト (テンポラリリポジトリ)

```rust
// tests/integration/git_operations.rs
use tempfile::TempDir;
use std::process::Command;

fn setup_test_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    // 初期コミット
    std::fs::write(dir.path().join("README.md"), "# Test").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(dir.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    dir
}

#[tokio::test]
async fn test_create_worktree() {
    let repo_dir = setup_test_repo();
    let repo = Repository::open(repo_dir.path()).unwrap();

    repo.create_branch("feature/test").unwrap();
    let wt_path = repo_dir.path().join(".worktrees/feature-test");
    repo.add_worktree("feature/test", &wt_path).unwrap();

    assert!(wt_path.exists());
}
```

### 5.2 Benchmarks (criterion)

```rust
// benches/git_operations.rs
use criterion::{criterion_group, criterion_main, Criterion};

fn bench_branch_list(c: &mut Criterion) {
    let repo = setup_large_repo();

    c.bench_function("list 1000 branches", |b| {
        b.iter(|| repo.list_branches())
    });
}

criterion_group!(benches, bench_branch_list);
criterion_main!(benches);
```

### 5.3 CI/CD (GitHub Actions、ネイティブランナー)

```yaml
# .github/workflows/ci.yml
name: CI

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test --all

  build:
    strategy:
      matrix:
        include:
          - os: ubuntu-latest
            target: x86_64-unknown-linux-gnu
          - os: ubuntu-latest
            target: aarch64-unknown-linux-gnu
          - os: macos-latest
            target: x86_64-apple-darwin
          - os: macos-latest
            target: aarch64-apple-darwin
          - os: windows-latest
            target: x86_64-pc-windows-msvc
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}
      - run: cargo build --release --target ${{ matrix.target }}
      - uses: actions/upload-artifact@v4
        with:
          name: gwt-${{ matrix.target }}
          path: target/${{ matrix.target }}/release/gwt*

  benchmark:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo bench
```

### 5.4 npm配布 (postinstall)

```javascript
// npm/postinstall.js
const https = require('https');
const fs = require('fs');
const { execSync } = require('child_process');

const PLATFORMS = {
    'darwin-x64': 'x86_64-apple-darwin',
    'darwin-arm64': 'aarch64-apple-darwin',
    'linux-x64': 'x86_64-unknown-linux-gnu',
    'linux-arm64': 'aarch64-unknown-linux-gnu',
    'win32-x64': 'x86_64-pc-windows-msvc',
};

const platform = `${process.platform}-${process.arch}`;
const target = PLATFORMS[platform];

if (!target) {
    console.error(`Unsupported platform: ${platform}`);
    process.exit(1);
}

const version = require('./package.json').version;
const url = `https://github.com/akiojin/gwt/releases/download/v${version}/gwt-${target}${process.platform === 'win32' ? '.exe' : ''}`;

// ダウンロード処理...
```

## 実装優先順位

### 最優先（Phase 1）

1. プロジェクト構造
2. エラー型（thiserror + コード）
3. Git操作（gix + フォールバック、git必須チェック）
4. 設定管理（TOML + マイグレーション）
5. ログシステム（JSON Lines + スパン）
6. ファイルロック

### 高優先（Phase 2）

1. Elmアーキテクチャ
2. 画面スタック
3. ブランチ一覧画面（遅延読み込み）
4. Ctrl+C二度押し終了
5. オフライン表示

### 中優先（Phase 3-4）

1. Coding Agent統合
2. Axum サーバー
3. Leptos フロントエンド

### 低優先（Phase 5）

1. 統合テスト
2. ベンチマーク
3. CI/CD
4. 配布

## 検証方法

### Phase完了基準

| Phase | 基準 |
| ----- | ---- |
| 1 | git/worktree操作がCLI動作、設定マイグレーション完了 |
| 2 | TUIで全画面動作、Ctrl+C二度押し終了 |
| 3 | Coding Agent起動・待機・終了が動作 |
| 4 | Web UIからWorktree操作が可能 |
| 5 | 全プラットフォームでバイナリ動作、npm配布 |

### 互換性検証

```bash
# 既存設定マイグレーション
cp ~/.gwt.json /tmp/
./gwt  # .gwt.toml が生成されること

# ログ互換性
cat ~/.gwt/logs/gwt.jsonl | jq  # JSON Lines + スパン情報

# キーバインド
# 矢印キー、Enter、Esc、q、PageUp/Down が動作すること
```
