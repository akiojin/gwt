# データモデル設計: ブランチサマリーパネル（セッション要約対応）

**仕様ID**: `SPEC-4b893dae` | **日付**: 2026-01-19 | **更新日**: 2026-01-19

## 概要

ブランチサマリーパネルで使用するデータ構造を定義する。既存の構造体を拡張し、新規構造体は最小限に抑える。

## エンティティ一覧

```text
┌─────────────────────────────────────────────────────────────┐
│                     BranchSummary                            │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐       │
│  │ commits:     │  │ stats:       │  │ meta:        │       │
│  │ Vec<Commit>  │  │ ChangeStats  │  │ BranchMeta   │       │
│  └──────────────┘  └──────────────┘  └──────────────┘       │
│  ┌──────────────┐  ┌──────────────┐                         │
│  │ ai_summary:  │  │ loading:     │                         │
│  │ Option<Vec>  │  │ LoadingState │                         │
│  └──────────────┘  └──────────────┘                         │
└─────────────────────────────────────────────────────────────┘
```

## 1. BranchSummary（新規）

パネル全体のデータを保持するコンテナ構造体。

```rust
/// ブランチサマリーパネルのデータ
#[derive(Debug, Clone, Default)]
pub struct BranchSummary {
    /// ブランチ名
    pub branch_name: String,
    /// Worktreeパス
    pub worktree_path: Option<PathBuf>,
    /// コミットログ（最新順、最大5件）
    pub commits: Vec<CommitEntry>,
    /// 変更統計
    pub stats: Option<ChangeStats>,
    /// ブランチメタデータ
    pub meta: Option<BranchMeta>,
    /// AIサマリー（箇条書き、最大3行）
    pub ai_summary: Option<Vec<String>>,
    /// ローディング状態
    pub loading: LoadingState,
}
```

## 2. CommitEntry（新規）

個々のコミット情報。`git log --oneline`の出力に対応。

```rust
/// コミットエントリ
#[derive(Debug, Clone)]
pub struct CommitEntry {
    /// コミットハッシュ（7桁）
    pub hash: String,
    /// コミットメッセージ（1行目のみ）
    pub message: String,
}

impl CommitEntry {
    /// git log --oneline の出力行をパース
    /// 例: "a1b2c3d fix: update README"
    pub fn from_oneline(line: &str) -> Option<Self> {
        let parts: Vec<&str> = line.splitn(2, ' ').collect();
        if parts.len() == 2 {
            Some(Self {
                hash: parts[0].to_string(),
                message: parts[1].to_string(),
            })
        } else {
            None
        }
    }
}
```

## 3. ChangeStats（新規）

変更統計情報。既存の`has_changes`/`has_unpushed`と組み合わせて使用。

```rust
/// 変更統計
#[derive(Debug, Clone, Default)]
pub struct ChangeStats {
    /// 変更ファイル数
    pub files_changed: usize,
    /// 追加行数
    pub insertions: usize,
    /// 削除行数
    pub deletions: usize,
    /// 未コミット変更あり（既存BranchItemから取得）
    pub has_uncommitted: bool,
    /// 未プッシュコミットあり（既存BranchItemから取得）
    pub has_unpushed: bool,
}

impl ChangeStats {
    /// git diff --shortstat の出力をパース
    /// 例: " 5 files changed, 120 insertions(+), 45 deletions(-)"
    pub fn from_shortstat(line: &str) -> Self {
        // パースロジック
    }
}
```

## 4. BranchMeta（新規）

ブランチメタデータ。既存のBranch構造体から取得。

```rust
/// ブランチメタデータ
#[derive(Debug, Clone)]
pub struct BranchMeta {
    /// upstream名（例: "origin/main"）
    pub upstream: Option<String>,
    /// upstreamより先行するコミット数
    pub ahead: usize,
    /// upstreamより遅延するコミット数
    pub behind: usize,
    /// 最終コミット日時（Unixタイムスタンプ）
    pub last_commit_timestamp: Option<i64>,
    /// ベースブランチ（例: "main"）
    pub base_branch: Option<String>,
}

impl BranchMeta {
    /// 既存のBranch構造体から変換
    pub fn from_branch(branch: &Branch) -> Self {
        Self {
            upstream: branch.upstream.clone(),
            ahead: branch.ahead,
            behind: branch.behind,
            last_commit_timestamp: branch.commit_timestamp,
            base_branch: None, // 別途取得
        }
    }

    /// 相対日時を文字列で取得
    /// 例: "2 days ago", "5 hours ago"
    pub fn relative_time(&self) -> Option<String> {
        // chrono使用で計算
    }
}
```

## 5. LoadingState（新規）

各セクションのローディング状態を管理。

```rust
/// ローディング状態
#[derive(Debug, Clone, Default)]
pub struct LoadingState {
    /// コミットログ取得中
    pub commits: bool,
    /// 変更統計取得中
    pub stats: bool,
    /// メタデータ取得中
    pub meta: bool,
    /// AIサマリー生成中
    pub ai_summary: bool,
}

impl LoadingState {
    /// いずれかがローディング中か
    pub fn is_any_loading(&self) -> bool {
        self.commits || self.stats || self.meta || self.ai_summary
    }
}
```

## 6. AISettings（Profile拡張）

既存のProfile構造体に追加するAI設定。

```rust
/// AI設定（Profile構造体に追加）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AISettings {
    /// APIエンドポイント
    #[serde(default = "default_endpoint")]
    pub endpoint: String,
    /// APIキー（空の場合は環境変数からフォールバック）
    #[serde(default)]
    pub api_key: String,
    /// モデル名
    #[serde(default = "default_model")]
    pub model: String,
}

fn default_endpoint() -> String {
    "https://api.openai.com/v1".to_string()
}

fn default_model() -> String {
    "gpt-4o-mini".to_string()
}
```

## 7. Profile構造体の変更

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Profile {
    pub name: String,
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub disabled_env: Vec<String>,
    #[serde(default)]
    pub description: String,
    // 新規追加
    #[serde(default)]
    pub ai: Option<AISettings>,
}
```

## 8. AISummaryCache（新規）

AIサマリーのメモリキャッシュ。

```rust
/// AIサマリーキャッシュ
#[derive(Debug, Default)]
pub struct AISummaryCache {
    /// ブランチ名 → サマリー（箇条書きリスト）
    cache: HashMap<String, Vec<String>>,
}

impl AISummaryCache {
    pub fn get(&self, branch: &str) -> Option<&Vec<String>> {
        self.cache.get(branch)
    }

    pub fn set(&mut self, branch: String, summary: Vec<String>) {
        self.cache.insert(branch, summary);
    }

    pub fn clear(&mut self) {
        self.cache.clear();
    }
}
```

## 関係図

```text
Profile
  └── ai: Option<AISettings>
          ├── endpoint: String
          ├── api_key: String
          └── model: String

BranchListState（既存）
  ├── branches: Vec<BranchItem>（既存）
  │       ├── has_changes: bool ──┐
  │       └── has_unpushed: bool ─┼──▶ ChangeStats
  │                               │
  └── branch_summary: Option<BranchSummary>（新規追加）
          ├── commits: Vec<CommitEntry>
          ├── stats: Option<ChangeStats>
          ├── meta: Option<BranchMeta>
          ├── ai_summary: Option<Vec<String>>
          └── loading: LoadingState

Branch（gwt-core、既存）
  ├── ahead: usize ────────┐
  ├── behind: usize ───────┼──▶ BranchMeta
  ├── commit_timestamp ────┘
  └── upstream: Option<String>
```

## バリデーションルール

| エンティティ              | ルール                   |
| ------------------------- | ------------------------ |
| CommitEntry.hash          | 7文字以上の16進数文字列  |
| CommitEntry.message       | 空文字列でない           |
| ChangeStats.files_changed | 0以上                    |
| AISettings.endpoint       | 有効なURL形式            |
| AISettings.model          | 空文字列でない           |

## ファイル配置

| 構造体         | ファイル                                         |
| -------------- | ------------------------------------------------ |
| BranchSummary  | `crates/gwt-cli/src/tui/screens/branch_list.rs`  |
| CommitEntry    | `crates/gwt-core/src/git/commit.rs`（新規）      |
| ChangeStats    | `crates/gwt-core/src/git/commit.rs`（新規）      |
| BranchMeta     | `crates/gwt-core/src/git/commit.rs`（新規）      |
| LoadingState   | `crates/gwt-cli/src/tui/screens/branch_list.rs`  |
| AISettings     | `crates/gwt-core/src/config/profile.rs`          |
| AISummaryCache | `crates/gwt-core/src/ai/summary.rs`（新規）      |

## 9. セッション要約関連エンティティ（追加）

### 9.1 DetailPanelTab（新規）

パネルのタブ状態を管理。

```rust
/// 詳細パネルのタブ
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DetailPanelTab {
    /// ブランチ詳細（コミット履歴、統計、メタデータ）
    #[default]
    Details,
    /// セッション要約
    Session,
}

impl DetailPanelTab {
    /// Tabキーでトグル
    pub fn toggle(&mut self) {
        *self = match self {
            DetailPanelTab::Details => DetailPanelTab::Session,
            DetailPanelTab::Session => DetailPanelTab::Details,
        };
    }
}
```

### 9.2 SessionSummary（新規）

セッション要約の完全なデータ構造。

```rust
/// セッション要約
#[derive(Debug, Clone, Default)]
pub struct SessionSummary {
    /// タスク概要（現在のタスクと進捗）
    pub task_overview: Option<String>,
    /// 短文要約（1-2文）
    pub short_summary: Option<String>,
    /// バレットポイント一覧（2-3件）
    pub bullet_points: Vec<String>,
    /// セッションメトリクス
    pub metrics: SessionMetrics,
    /// 最終更新時刻
    pub last_updated: Option<std::time::SystemTime>,
}
```

### 9.3 SessionMetrics（新規）

セッションのメトリクス情報。

```rust
/// セッションメトリクス
#[derive(Debug, Clone, Default)]
pub struct SessionMetrics {
    /// 推定トークン数
    pub token_count: Option<usize>,
    /// ツール実行回数
    pub tool_execution_count: usize,
    /// セッション経過時間（秒）
    pub elapsed_seconds: Option<u64>,
    /// ターン数（user + assistant）
    pub turn_count: usize,
}
```

### 9.4 ParsedSession（新規）

パース済みセッションデータ。

```rust
/// パース済みセッション
#[derive(Debug, Clone)]
pub struct ParsedSession {
    /// セッションID
    pub session_id: String,
    /// エージェント種別
    pub agent_type: AgentType,
    /// 会話履歴（サンプリング済み）
    pub messages: Vec<SessionMessage>,
    /// ツール実行履歴
    pub tool_executions: Vec<ToolExecution>,
    /// セッション開始時刻
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    /// 最終更新時刻
    pub last_updated_at: Option<chrono::DateTime<chrono::Utc>>,
}
```

### 9.5 SessionMessage（新規）

セッション内の個々のメッセージ。

```rust
/// セッションメッセージ
#[derive(Debug, Clone)]
pub struct SessionMessage {
    /// ロール（user / assistant）
    pub role: MessageRole,
    /// コンテンツ（テキスト）
    pub content: String,
    /// タイムスタンプ（あれば）
    pub timestamp: Option<chrono::DateTime<chrono::Utc>>,
}

/// メッセージロール
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Assistant,
}
```

### 9.6 ToolExecution（新規）

ツール実行の記録。

```rust
/// ツール実行
#[derive(Debug, Clone)]
pub struct ToolExecution {
    /// ツール名（Read, Edit, Bash等）
    pub tool_name: String,
    /// 実行成功/失敗
    pub success: bool,
    /// タイムスタンプ（あれば）
    pub timestamp: Option<chrono::DateTime<chrono::Utc>>,
}
```

### 9.7 AgentType（新規）

対応エージェントの種別。

```rust
/// エージェント種別
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentType {
    ClaudeCode,
    CodexCli,
    GeminiCli,
    OpenCode,
}

impl AgentType {
    /// 表示名を取得
    pub fn display_name(&self) -> &'static str {
        match self {
            AgentType::ClaudeCode => "Claude Code",
            AgentType::CodexCli => "Codex CLI",
            AgentType::GeminiCli => "Gemini CLI",
            AgentType::OpenCode => "OpenCode",
        }
    }
}
```

### 9.8 SessionParserTrait（新規）

セッションパーサーの共通インターフェース。

```rust
/// セッションパーサートレイト
pub trait SessionParser: Send + Sync {
    /// セッションファイルをパース
    fn parse(&self, session_id: &str) -> Result<ParsedSession, SessionParseError>;

    /// 対応エージェント種別
    fn agent_type(&self) -> AgentType;

    /// セッションファイルのパスを取得
    fn session_file_path(&self, session_id: &str) -> PathBuf;

    /// セッションファイルが存在するか確認
    fn session_exists(&self, session_id: &str) -> bool {
        self.session_file_path(session_id).exists()
    }
}
```

### 9.9 SessionParseError（新規）

パースエラーの種類。

```rust
/// セッションパースエラー
#[derive(Debug, thiserror::Error)]
pub enum SessionParseError {
    #[error("Session file not found: {0}")]
    FileNotFound(String),
    #[error("Invalid session format: {0}")]
    InvalidFormat(String),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("JSON parse error: {0}")]
    JsonError(#[from] serde_json::Error),
}
```

### 9.10 SessionSummaryCache（新規）

セッション要約のキャッシュ。

```rust
/// セッション要約キャッシュ
#[derive(Debug, Default)]
pub struct SessionSummaryCache {
    /// ブランチ名 → 要約
    cache: HashMap<String, SessionSummary>,
    /// ブランチ名 → セッションファイル最終更新時刻
    last_modified: HashMap<String, std::time::SystemTime>,
}

impl SessionSummaryCache {
    pub fn get(&self, branch: &str) -> Option<&SessionSummary> {
        self.cache.get(branch)
    }

    pub fn set(&mut self, branch: String, summary: SessionSummary, mtime: std::time::SystemTime) {
        self.cache.insert(branch.clone(), summary);
        self.last_modified.insert(branch, mtime);
    }

    /// ファイル更新時刻が変わっているか確認
    pub fn is_stale(&self, branch: &str, current_mtime: std::time::SystemTime) -> bool {
        self.last_modified
            .get(branch)
            .map(|&cached| cached < current_mtime)
            .unwrap_or(true)
    }
}
```

## 10. BranchSummary拡張

タブ切り替えとセッション要約に対応。

```rust
/// ブランチサマリーパネルのデータ（拡張版）
#[derive(Debug, Clone, Default)]
pub struct BranchSummary {
    /// ブランチ名
    pub branch_name: String,
    /// Worktreeパス
    pub worktree_path: Option<PathBuf>,
    /// コミットログ（最新順、最大5件）
    pub commits: Vec<CommitEntry>,
    /// 変更統計
    pub stats: Option<ChangeStats>,
    /// ブランチメタデータ
    pub meta: Option<BranchMeta>,
    /// セッション要約（タブ切り替え対応）
    pub session_summary: Option<SessionSummary>,
    /// 現在のタブ
    pub current_tab: DetailPanelTab,
    /// ローディング状態
    pub loading: LoadingState,
}
```

## 11. LoadingState拡張

セッション要約のローディング状態を追加。

```rust
/// ローディング状態（拡張版）
#[derive(Debug, Clone, Default)]
pub struct LoadingState {
    /// コミットログ取得中
    pub commits: bool,
    /// 変更統計取得中
    pub stats: bool,
    /// メタデータ取得中
    pub meta: bool,
    /// セッション要約生成中
    pub session_summary: bool,
}
```

## セッション要約関連ファイル配置

| 構造体               | ファイル                                              |
| -------------------- | ----------------------------------------------------- |
| DetailPanelTab       | `crates/gwt-cli/src/tui/screens/branch_list.rs`       |
| SessionSummary       | `crates/gwt-core/src/ai/session_summary.rs`（新規）   |
| SessionMetrics       | `crates/gwt-core/src/ai/session_summary.rs`（新規）   |
| ParsedSession        | `crates/gwt-core/src/ai/session_parser/mod.rs`（新規）|
| SessionMessage       | `crates/gwt-core/src/ai/session_parser/mod.rs`（新規）|
| ToolExecution        | `crates/gwt-core/src/ai/session_parser/mod.rs`（新規）|
| AgentType            | `crates/gwt-core/src/ai/session_parser/mod.rs`（新規）|
| SessionParser trait  | `crates/gwt-core/src/ai/session_parser/mod.rs`（新規）|
| ClaudeCodeParser     | `crates/gwt-core/src/ai/session_parser/claude.rs`（新規）|
| CodexCliParser       | `crates/gwt-core/src/ai/session_parser/codex.rs`（新規）|
| GeminiCliParser      | `crates/gwt-core/src/ai/session_parser/gemini.rs`（新規）|
| OpenCodeParser       | `crates/gwt-core/src/ai/session_parser/opencode.rs`（新規）|
| SessionSummaryCache  | `crates/gwt-core/src/ai/session_summary.rs`（新規）   |
