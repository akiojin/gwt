# クイックスタート: エージェントモード開発

**仕様ID**: `SPEC-ba3f610c` | **日付**: 2026-01-22

## 前提条件

- Rust stable (1.75+)
- tmux 3.0+
- Claude Code または他のコーディングエージェント（テスト用）

## セットアップ

### 1. リポジトリのクローン

```bash
git clone https://github.com/akiojin/gwt.git
cd gwt
```

### 2. ビルド

```bash
cargo build --release
```

### 3. AI設定

既存のAI要約機能と同じ設定を使用します。

```bash
# 設定ファイルの場所
~/.gwt/config.toml

# または環境変数
export OPENAI_API_KEY="your-api-key"
```

## 開発ワークフロー

### 新規モジュールの追加

1. `gwt-core/src/agent/`にファイルを追加
2. `gwt-core/src/agent/mod.rs`でエクスポート
3. `gwt-core/src/lib.rs`でモジュールを公開

```rust
// gwt-core/src/lib.rs
pub mod agent;

// gwt-core/src/agent/mod.rs
pub mod master;
pub mod task;
pub mod session;
pub mod orchestrator;

pub use master::MasterAgent;
pub use task::{Task, TaskId, TaskStatus};
pub use session::AgentSession;
```

### TUI画面の追加

1. `gwt-cli/src/tui/screens/agent_mode.rs`を作成
2. `gwt-cli/src/tui/screens/mod.rs`でエクスポート
3. `gwt-cli/src/tui/app.rs`にモード切り替えロジックを追加

```rust
// screens/mod.rs
pub mod agent_mode;
pub use agent_mode::{render_agent_mode, AgentModeState};

// app.rs - キーハンドリング
KeyCode::Tab => {
    // Toggle agent mode
    self.toggle_agent_mode();
}
```

## テスト実行

### 全テスト

```bash
cargo test
```

### 特定モジュールのテスト

```bash
# agentモジュール
cargo test -p gwt-core agent::

# TUIテスト
cargo test -p gwt-cli
```

### 統合テスト

```bash
cargo test --test integration
```

## Lint & フォーマット

```bash
# フォーマット
cargo fmt

# Lint
cargo clippy --all-targets --all-features -- -D warnings
```

## デバッグ

### ログ出力

```bash
# 環境変数でログレベルを設定
RUST_LOG=debug cargo run

# 特定モジュールのログ
RUST_LOG=gwt_core::agent=trace cargo run
```

### tmuxデバッグ

```bash
# ペイン一覧
tmux list-panes -a

# ペイン出力をキャプチャ
tmux capture-pane -p -t %1
```

## ディレクトリ構造

```text
crates/
├── gwt-cli/
│   └── src/
│       └── tui/
│           ├── app.rs              # メインアプリ
│           └── screens/
│               ├── mod.rs
│               └── agent_mode.rs   # 新規
└── gwt-core/
    └── src/
        ├── agent/                  # 新規モジュール
        │   ├── mod.rs
        │   ├── master.rs           # マスターエージェント
        │   ├── task.rs             # タスク管理
        │   ├── session.rs          # セッション永続化
        │   └── orchestrator.rs     # オーケストレーション
        ├── ai/
        │   └── client.rs           # 既存（再利用）
        └── tmux/
            └── pane.rs             # 既存（拡張）
```

## よくある操作

### セッションファイルの確認

```bash
# セッション一覧
ls ~/.gwt/sessions/

# セッション内容
cat ~/.gwt/sessions/<session-id>.json | jq .
```

### Worktreeの確認

```bash
# gwtが作成したworktree一覧
git worktree list | grep agent/
```

### Claude Code Hook設定の確認

```bash
# Hook設定ファイル
cat ~/.claude/settings.json | jq .hooks
```

## トラブルシューティング

### tmuxが見つからない

```bash
# インストール
# macOS
brew install tmux

# Ubuntu/Debian
sudo apt install tmux
```

### AI APIエラー

```bash
# API設定を確認
gwt config show

# 環境変数を確認
echo $OPENAI_API_KEY
```

### ペインが作成されない

```bash
# tmuxセッション内で実行しているか確認
echo $TMUX

# tmuxを起動
tmux new-session -s gwt
```

## 関連ドキュメント

- [仕様書](./spec.md)
- [実装計画](./plan.md)
- [技術調査](./research.md)
- [データモデル](./data-model.md)
