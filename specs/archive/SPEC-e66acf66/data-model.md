# データモデル: エラーポップアップ・ログ出力システム

**仕様ID**: `SPEC-e66acf66` | **日付**: 2026-01-22

## エンティティ一覧

### 1. ErrorCode

エラーの一意識別子。GWT-EXXX形式。

```rust
/// エラーコード（GWT-EXXX形式）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    // Worktree関連 (E001-E010)
    WorktreeCreateFailed,    // GWT-E001
    WorktreeDeleteFailed,    // GWT-E002
    WorktreeNotFound,        // GWT-E003

    // Git関連 (E011-E020)
    GitCommandFailed,        // GWT-E011
    BranchNotFound,          // GWT-E012
    MergeConflict,           // GWT-E013

    // 設定関連 (E021-E030)
    ConfigSaveFailed,        // GWT-E021
    ConfigLoadFailed,        // GWT-E022
    ProfileNotFound,         // GWT-E023

    // AI関連 (E031-E040)
    AiApiCallFailed,         // GWT-E031
    AiApiKeyMissing,         // GWT-E032
    AiResponseInvalid,       // GWT-E033

    // システム関連 (E041-E050)
    FileOperationFailed,     // GWT-E041
    PermissionDenied,        // GWT-E042
    NetworkError,            // GWT-E043

    // その他
    Unknown,                 // GWT-E999
}

impl ErrorCode {
    /// コード文字列を取得（例: "GWT-E001"）
    pub fn as_str(&self) -> &'static str;

    /// カテゴリを取得
    pub fn category(&self) -> ErrorCategory;

    /// サジェスチョンを取得
    pub fn suggestions(&self) -> Vec<&'static str>;
}
```

### 2. ErrorCategory

エラーの分類。

```rust
/// エラーカテゴリ
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCategory {
    Worktree,
    Git,
    Config,
    Ai,
    System,
    Unknown,
}

impl ErrorCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Worktree => "worktree",
            Self::Git => "git",
            Self::Config => "config",
            Self::Ai => "ai",
            Self::System => "system",
            Self::Unknown => "unknown",
        }
    }
}
```

### 3. ErrorSeverity

エラーの重大度。（既存）

```rust
/// エラーの重大度
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ErrorSeverity {
    #[default]
    Error,   // 赤
    Warning, // 黄
    Info,    // 青
}

impl ErrorSeverity {
    pub fn color(&self) -> Color {
        match self {
            Self::Error => Color::Red,
            Self::Warning => Color::Yellow,
            Self::Info => Color::Blue,
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Self::Error => "[!]",
            Self::Warning => "[*]",
            Self::Info => "[i]",
        }
    }
}
```

### 4. ErrorState

表示用エラー状態。（既存を拡張）

```rust
/// エラー状態（ポップアップ表示用）
#[derive(Debug, Clone, Default)]
pub struct ErrorState {
    /// エラーコード（GWT-EXXX）
    pub code: Option<ErrorCode>,

    /// タイトル（ポップアップヘッダー）
    pub title: String,

    /// メッセージ（本文）
    pub message: String,

    /// 詳細情報（スクロール可能セクション）
    pub details: Option<String>,

    /// サジェスチョン（解決策リスト）
    pub suggestions: Option<Vec<String>>,

    /// 重大度
    pub severity: ErrorSeverity,
}

impl ErrorState {
    /// エラーメッセージからErrorStateを生成
    pub fn from_error(message: &str) -> Self;

    /// ErrorCodeからErrorStateを生成
    pub fn from_code(code: ErrorCode, message: String, details: Option<String>) -> Self;

    /// 表示状態かどうか
    pub fn is_visible(&self) -> bool {
        !self.message.is_empty()
    }

    /// JSON形式でエクスポート（クリップボード用）
    pub fn to_json(&self) -> String;
}
```

### 5. ErrorQueue

複数エラーのFIFOキュー。

```rust
use std::collections::VecDeque;

/// エラーキュー（複数エラーの順次処理用）
#[derive(Debug, Default)]
pub struct ErrorQueue {
    /// エラーのキュー
    errors: VecDeque<ErrorState>,

    /// 現在表示中のエラー
    current: Option<ErrorState>,
}

impl ErrorQueue {
    /// 新規作成
    pub fn new() -> Self;

    /// エラーを追加
    pub fn push(&mut self, error: ErrorState);

    /// 現在のエラーを閉じて次へ
    pub fn dismiss_current(&mut self);

    /// 現在のエラーを取得
    pub fn current(&self) -> Option<&ErrorState>;

    /// キューが空かどうか
    pub fn is_empty(&self) -> bool;

    /// キューの長さ（現在のエラー含む）
    pub fn total_count(&self) -> usize;

    /// 現在の位置（1-indexed）
    pub fn current_position(&self) -> usize;

    /// 位置文字列を取得（例: "(1/3)"）
    pub fn position_string(&self) -> Option<String>;
}
```

## リレーション

```text
ErrorCode ───────────────────┐
    │                        │
    ├── category() ──► ErrorCategory
    │                        │
    └── suggestions() ──► Vec<&str>

ErrorState ◄────────────────┘
    │
    ├── code: Option<ErrorCode>
    ├── severity: ErrorSeverity
    └── to_json() ──► String (clipboard)

ErrorQueue
    │
    └── errors: VecDeque<ErrorState>
```

## ログ出力形式

### JSON Lines フォーマット

```json
{
  "timestamp": "2026-01-22T12:34:56.789Z",
  "level": "ERROR",
  "target": "gwt_core::error",
  "message": "Error occurred",
  "code": "GWT-E001",
  "category": "worktree",
  "error_message": "Failed to create worktree",
  "details": "Branch 'feature/foo' does not exist"
}
```

### クリップボードエクスポート形式

```json
{
  "code": "GWT-E001",
  "category": "worktree",
  "severity": "error",
  "title": "Worktree Creation Failed",
  "message": "Failed to create worktree for branch 'feature/foo'",
  "details": "Branch 'feature/foo' does not exist in remote",
  "suggestions": [
    "Check if branch exists: git branch -a | grep feature/foo",
    "Fetch latest branches: git fetch --all"
  ]
}
```

## バリデーションルール

| エンティティ | フィールド | ルール |
|-------------|----------|--------|
| ErrorState | message | 空文字列不可（is_visible判定） |
| ErrorState | title | 最大70文字（ポップアップ幅制限） |
| ErrorQueue | errors | 最大100件（古いものから削除） |
