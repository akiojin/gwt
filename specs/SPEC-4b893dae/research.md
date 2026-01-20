# 調査報告: ブランチサマリーパネル

**仕様ID**: `SPEC-4b893dae` | **日付**: 2026-01-19

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
| HTTPクライアント   | reqwest（新規追加）     |
| シリアライズ       | serde_yaml, serde_json  |
| Git操作            | std::process::Command   |
