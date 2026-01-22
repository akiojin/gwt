# 調査結果: エラーポップアップ・ログ出力システム

**仕様ID**: `SPEC-e66acf66` | **日付**: 2026-01-22

## 1. 既存のコードベース分析

### 1.1 現在のエラー表示

**ファイル**: `crates/gwt-cli/src/tui/screens/error.rs`

```rust
pub struct ErrorState {
    pub title: String,
    pub message: String,
    pub code: Option<String>,
    pub details: Option<String>,
    pub suggestions: Option<Vec<String>>,
    pub severity: ErrorSeverity,
}

pub enum ErrorSeverity {
    Error,
    Warning,
    Info,
}

pub fn render_error(f: &mut Frame, area: Rect, state: &ErrorState)
```

**現状**:
- ErrorStateは既にcode, details, suggestions, severityフィールドを持つ
- render_error関数でポップアップ表示を実装済み
- 拡張ポイント: キュー機能、キーボード/マウスハンドリング

### 1.2 エラー発生箇所

**ファイル**: `crates/gwt-cli/src/tui/app.rs`

主なエラー発生箇所:
- L1381: `self.error = ErrorState::from_error(&message);`
- Worktree作成失敗
- Gitコマンド実行失敗
- プロファイル設定保存失敗
- AI機能エラー

### 1.3 既存のログシステム

**ファイル**: `crates/gwt-core/src/logging/`

```rust
// logger.rs
pub fn init_logger() -> Result<WorkerGuard>
// - tracing-appenderでJSON Lines出力
// - ファイル: ~/.gwt/logs/gwt.jsonl.YYYY-MM-DD

// reader.rs
pub struct LogReader { ... }
pub struct LogEntry {
    pub timestamp: DateTime<Utc>,
    pub level: String,
    pub message: String,
    pub target: String,
    pub fields: HashMap<String, Value>,
}
```

**現状**:
- tracingマクロ（info!, error!等）でログ出力
- JSON Lines形式でファイルに保存
- LogReaderでログファイル読み込み可能
- **課題**: 現在のエラーはログに出力されていない（error!マクロ未使用）

### 1.4 ログビューアー

**ファイル**: `crates/gwt-cli/src/tui/screens/logs.rs`

```rust
pub struct LogsState {
    pub entries: Vec<LogEntry>,
    pub selected_index: usize,
    pub filter_level: Option<String>,
    pub search_query: String,
}
```

- ログファイルから読み込んで表示
- レベルフィルタ、検索機能あり

## 2. 技術的決定

### 2.1 クリップボード

**決定**: `arboard` クレートを使用

**理由**:
- cli-clipboardよりも活発にメンテナンスされている
- macOS, Linux (X11/Wayland), Windowsに対応
- 依存関係が少ない

```toml
[dependencies]
arboard = "3"
```

### 2.2 エラーキュー

**決定**: `VecDeque<ErrorState>` で実装

```rust
use std::collections::VecDeque;

pub struct ErrorQueue {
    errors: VecDeque<ErrorState>,
}

impl ErrorQueue {
    pub fn push(&mut self, error: ErrorState);
    pub fn pop(&mut self) -> Option<ErrorState>;
    pub fn current(&self) -> Option<&ErrorState>;
    pub fn len(&self) -> usize;
    pub fn position(&self) -> (usize, usize); // (current, total)
}
```

### 2.3 マウスイベント

**決定**: crosstermのMouseEventで処理

```rust
use crossterm::event::{MouseEvent, MouseEventKind, MouseButton};

// ポップアップ外クリック検出
fn is_outside_popup(mouse_event: &MouseEvent, popup_area: Rect) -> bool {
    let (x, y) = (mouse_event.column, mouse_event.row);
    !popup_area.contains(Position::new(x, y))
}
```

### 2.4 エラーコード体系

**決定**: GWT-EXXX形式

| コード | カテゴリ | 説明 |
|--------|----------|------|
| GWT-E001 | worktree | Worktree作成失敗 |
| GWT-E002 | worktree | Worktree削除失敗 |
| GWT-E003 | git | Gitコマンド実行失敗 |
| GWT-E004 | git | ブランチが存在しない |
| GWT-E005 | config | プロファイル保存失敗 |
| GWT-E006 | config | 設定ファイル読み込み失敗 |
| GWT-E007 | ai | AI API呼び出し失敗 |
| GWT-E008 | ai | APIキー未設定 |
| GWT-E009 | system | ファイル操作失敗 |
| GWT-E010 | system | 権限エラー |

### 2.5 エラーログ出力

**決定**: 既存のtracingシステムを使用

```rust
use tracing::error;

// エラー発生時にログ出力
error!(
    code = %error_code,
    category = %category,
    message = %message,
    details = ?details,
    "Error occurred"
);
```

## 3. 制約と依存関係

### 3.1 制約

- **ASCII文字のみ**: アイコンは `[!]`, `[i]`, `[*]` 等を使用
- **既存API互換**: ErrorState::from_error()の署名を維持
- **ポップアップ幅**: 最大70文字

### 3.2 依存関係

新規追加:
- `arboard = "3"` - クリップボード操作

既存（変更なし）:
- `tracing`, `tracing-appender` - ログ出力
- `crossterm` - ターミナル入力
- `ratatui` - TUI描画

## 4. 実装方針

### 4.1 ファイル構成

```text
crates/gwt-core/src/
├── error/
│   ├── mod.rs          # モジュール定義
│   ├── codes.rs        # ErrorCode enum
│   └── suggestions.rs  # サジェスチョンマッピング
└── logging/
    └── logger.rs       # error!マクロ呼び出し追加

crates/gwt-cli/src/tui/
├── screens/
│   └── error.rs        # ErrorQueue追加、キー/マウスハンドリング
└── app.rs              # エラー発生時にキュー追加・ログ出力
```

### 4.2 実装順序

1. **Phase 1**: ErrorCode enum + サジェスチョン（gwt-core）
2. **Phase 2**: ErrorQueue実装（gwt-cli）
3. **Phase 3**: エラー発生時のログ出力（gwt-core/app.rs連携）
4. **Phase 4**: ポップアップUI拡張（キーボード/マウス）
5. **Phase 5**: クリップボードコピー機能
6. **Phase 6**: ログ画面遷移機能

## 5. 結論

既存のコードベースは十分な拡張ポイントを持っており、大規模なリファクタリングなしに要件を実装可能。主な作業は:

1. ErrorCodeの新規作成とサジェスチョンマッピング
2. ErrorQueueの追加
3. エラー発生箇所でのerror!マクロ呼び出し追加
4. render_errorの拡張（キュー位置表示、キー/マウスハンドリング）
