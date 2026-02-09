# 調査報告: ブランチサマリーパネル（セッション要約対応）

**仕様ID**: `SPEC-4b893dae` | **日付**: 2026-01-19 | **更新日**: 2026-01-19

## 1. 既存コードベース分析

### 1.1 TUI実装構造

**ファイル構成**:

```text
crates/gwt-cli/src/tui/
├── app.rs                    (3,508行) - メインTUIアプリケーション
├── screens/
│   ├── branch_list.rs        (1,777行) - ブランチ一覧画面
│   └── ...
```

**現在のレイアウト構造** (`branch_list.rs:822-828`):

```rust
let chunks = Layout::default()
    .direction(Direction::Vertical)
    .constraints([
        Constraint::Min(3),        // ブランチリスト（最小3行）
        Constraint::Length(1),     // Worktreeパス or ステータス（1行）
    ])
    .split(area);
```

**現在のフッター表示** (`render_worktree_path`関数):
- ステータスメッセージ優先
- ローディング/プログレス表示
- `Worktree: <path>` 形式で選択ブランチのパスを表示

### 1.2 既存データ構造

**BranchItem**（ブランチ情報）:

```rust
pub struct BranchItem {
    pub name: String,
    pub branch_type: BranchType,
    pub is_current: bool,
    pub has_worktree: bool,
    pub worktree_path: Option<String>,     // ← 既存
    pub worktree_status: WorktreeStatus,
    pub has_changes: bool,                  // ← 既存（未コミット）
    pub has_unpushed: bool,                 // ← 既存（未プッシュ）
    pub safety_status: SafetyStatus,        // ← 既存（安全性判定）
    pub last_tool_usage: Option<String>,
}
```

**Branch構造体**（gwt-core）:

```rust
pub struct Branch {
    pub name: String,
    pub commit: String,                     // コミットSHA
    pub ahead: usize,                       // ahead数
    pub behind: usize,                      // behind数
    pub commit_timestamp: Option<i64>,      // 最終コミット時刻
    pub upstream: Option<String>,           // upstream名
    // ...
}
```

### 1.3 プロファイルシステム

**保存形式**: YAML (`~/.gwt/profiles.yaml`)

**Profile構造体**:

```rust
pub struct Profile {
    pub name: String,
    pub env: HashMap<String, String>,       // 環境変数
    pub disabled_env: Vec<String>,
    pub description: String,
}
```

**設定マージ順序**: デフォルト → ファイル → 環境変数

## 2. 技術的決定

### 2.1 コミットログ取得

**決定**: `git log --oneline -n 5` コマンドをラップ

**理由**:
- 既存パターンに準拠（`std::process::Command`でgitコマンド実行）
- gitoxide（gix）はWorktree対応が不完全
- パフォーマンス: シンプルなコマンドで十分高速

**実装場所**: `crates/gwt-core/src/git/repository.rs`

### 2.2 変更統計取得

**決定**: 既存の安全性判定データを再利用

**理由**:
- `has_changes`、`has_unpushed`は既に取得済み
- 追加で`git diff --shortstat`を実行してファイル数・行数を取得

**実装場所**: `crates/gwt-core/src/git/repository.rs`

### 2.3 AI設定

**決定**: Profileに`ai`セクションを追加

```yaml
profiles:
  default:
    name: default
    env: {}
    ai:
      endpoint: "https://api.openai.com/v1"
      api_key: ""  # 空の場合は環境変数からフォールバック
      model: "gpt-4o-mini"
```

**理由**:
- 既存のプロファイルシステムを拡張
- プロジェクトごとに異なるAI設定が可能
- 環境変数フォールバックで既存ワークフローを維持

### 2.4 パネルレイアウト

**決定**: フッター領域を12行固定のパネルに拡張

```rust
let chunks = Layout::default()
    .direction(Direction::Vertical)
    .constraints([
        Constraint::Min(3),         // ブランチリスト
        Constraint::Length(12),     // サマリーパネル（12行固定）
    ])
    .split(area);
```

**理由**:
- 10-15行の仕様範囲内
- 内訳: 枠線2行 + タイトル1行 + Commits 4行 + Stats 1行 + Meta 1行 + Summary 3行 = 12行

## 3. 制約と依存関係

### 3.1 制約

| 制約           | 詳細                                     |
| -------------- | ---------------------------------------- |
| Ratatui        | CLI UIフレームワーク、ASCII文字のみ      |
| パフォーマンス | パネル更新200ms以内                      |
| メモリ         | AIサマリーはセッション中キャッシュ       |
| ネットワーク   | AI機能はオプショナル（APIなしでも動作）  |

### 3.2 依存関係

| 依存           | 詳細                          |
| -------------- | ----------------------------- |
| SPEC-d2f4762a  | 安全性判定データの共有        |
| Branch構造体   | ahead/behind/commit_timestamp |
| Profile        | AI設定の保存                  |
| OpenAI互換API  | AIサマリー生成                |

## 4. 未実装機能の確認

| 機能             | 現状   | 必要な実装                           |
| ---------------- | ------ | ------------------------------------ |
| コミットログ取得 | 未実装 | `git log --oneline -n 5` ラッパー    |
| 変更統計（行数） | 未実装 | `git diff --shortstat` ラッパー      |
| AI API呼び出し   | 未実装 | reqwestでOpenAI互換API呼び出し       |
| AI設定           | 未実装 | Profile構造体にaiフィールド追加      |
| パネルUI         | 未実装 | 新規Ratatuiコンポーネント            |

## 5. リスク評価

### 高リスク

- **AI API依存**: ネットワーク障害時のUX低下
  - **緩和策**: AIセクションを非表示にし、他機能は正常動作

### 中リスク

- **パフォーマンス**: 大量ブランチでのパネル更新遅延
  - **緩和策**: バックグラウンド取得、キャッシュ活用

### 低リスク

- **レイアウト崩れ**: 狭いターミナルでの表示問題
  - **緩和策**: 最小幅チェック、末尾省略

## 6. 技術スタック確認

| 項目               | 値                      |
| ------------------ | ----------------------- |
| 言語               | Rust (Stable)           |
| TUIフレームワーク  | Ratatui                 |
| HTTPクライアント   | reqwest（既存）         |
| シリアライズ       | serde_yaml, serde_json  |
| Git操作            | std::process::Command   |

## 7. セッション要約機能の調査（追加）

### 7.1 AIクライアント (`crates/gwt-core/src/ai/client.rs`)

**概要**: OpenAI互換APIクライアント（reqwestブロッキング）

**主要コンポーネント**:

- `AIClient`: API呼び出しを担当
- `ChatMessage`: role + content構造
- `AIError`: Unauthorized, RateLimited, ServerError, NetworkError, ParseError, ConfigError

**再利用可能な点**:

- `create_chat_completion(messages)` メソッドをそのまま使用可能
- リトライロジック（Rate Limit、Server Error、Network Error）実装済み
- Azure OpenAI対応済み（`api-key`ヘッダー）

**変更が必要な点**:

- `MAX_TOKENS: u32 = 150` → セッション要約用に調整が必要（300-500程度）
- 要約用の新しいプロンプトテンプレートが必要

### 7.2 コミット要約 (`crates/gwt-core/src/ai/summary.rs`)

**概要**: コミットログをAIで要約

**主要コンポーネント**:

- `SYSTEM_PROMPT`: 2-3行箇条書きを要求
- `build_user_prompt()`: ブランチ名+コミットリストを整形
- `summarize_commits()`: APIコール+パース
- `parse_summary_lines()`: レスポンスを箇条書きに正規化
- `AISummaryCache`: ブランチ名→Vec<String>のHashMapキャッシュ

**再利用可能な点**:

- `parse_summary_lines()` はセッション要約でもそのまま使用可能
- `AISummaryCache` パターンを `SessionSummaryCache` に適用可能

### 7.3 セッションID検出 (`crates/gwt-cli/src/main.rs`)

**概要**: 4エージェントのセッションID検出

**主要関数**:

- `detect_claude_session_id_at(home, worktree_path)` → Option<String>
- `detect_codex_session_id_at(home, worktree_path)` → Option<String>
- `detect_gemini_session_id_at(home)` → Option<String>
- `detect_opencode_session_id_at(home)` → Option<String>
- `detect_agent_session_id(config)` → Option<String>

**検出対象ファイル**:

- Claude Code: `~/.claude/projects/*/` 配下のJSONLファイル
- Codex CLI: `~/.codex/sessions/` 配下
- Gemini CLI: `~/.gemini/sessions/` 配下
- OpenCode: `~/.opencode/sessions/` 配下

### 7.4 セッション管理 (`crates/gwt-core/src/config/ts_session.rs`)

**概要**: gwtのセッション履歴管理

**主要構造体**:

- `ToolSessionEntry`: branch, worktree_path, tool_id, session_id, timestamp等
- `TsSessionData`: last_branch, last_session_id, history[]

**再利用可能な点**:

- `session_id` フィールドでエージェントセッションを特定可能
- `tool_id` の正規化ロジック（`canonical_tool_id()`）

## 8. エージェントセッションファイル形式

### 8.1 Claude Code

**場所**: `~/.claude/projects/<project-hash>/<session-id>.jsonl`

**形式**: JSONL（1行1JSONオブジェクト）

**主要フィールド**:

```json
{"type": "user", "message": {"content": "..."}}
{"type": "assistant", "message": {"content": "..."}}
{"type": "tool_use", "name": "Read", "input": {...}}
{"type": "tool_result", "content": "..."}
```

### 8.2 Codex CLI

**場所**: `~/.codex/sessions/<session-id>.jsonl`

**形式**: JSONL（要実機確認）

### 8.3 Gemini CLI

**場所**: `~/.gemini/sessions/<session-id>.json`

**形式**: JSON（単一オブジェクト、messagesフィールド内に配列）

### 8.4 OpenCode

**場所**: `~/.opencode/sessions/<session-id>.json`

**形式**: JSON（要実機確認）

## 9. セッション要約の技術的決定

### 9.1 セッションパーサー設計

**決定**: 共通trait `SessionParser` + 各エージェント別実装

```rust
pub trait SessionParser: Send + Sync {
    fn parse(&self, session_id: &str) -> Result<ParsedSession, ParseError>;
    fn agent_type(&self) -> AgentType;
}
```

### 9.2 ポーリング実装

**決定**: `std::thread::spawn` + `mpsc::channel`

**理由**:

- 既存のgwt TUIはブロッキングI/Oベース
- 60秒間隔なので、高頻度ポーリングではない

### 9.3 キャッシュ戦略

**決定**: `SessionSummaryCache` をメモリ内HashMap実装

```rust
pub struct SessionSummaryCache {
    cache: HashMap<String, SessionSummary>,
    last_modified: HashMap<String, SystemTime>,
}
```

### 9.4 動的サンプリング

**決定**: セッション長に応じて3段階サンプリング

- 100ターン以下: 全件使用
- 101-1000ターン: 最初50 + 最後50
- 1001ターン以上: 最初30 + 中間20 + 最後30

## 10. 未確認事項

| 項目 | 状態 | 対応 |
|------|------|------|
| Codex CLI セッションファイル詳細形式 | 要確認 | 実機確認後にパーサー調整 |
| Gemini CLI セッションファイル詳細形式 | 要確認 | 実機確認後にパーサー調整 |
| OpenCode セッションファイル詳細形式 | 要確認 | 実機確認後にパーサー調整 |
