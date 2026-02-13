# 調査報告: GitView画面

**仕様ID**: `SPEC-1ea18899` | **日付**: 2026-02-02

## 1. 既存コードベース分析

### 1.1 Screen enum

**ファイル**: `crates/gwt-cli/src/tui/app.rs` (行526-543)

```rust
pub enum Screen {
    BranchList,
    AgentMode,
    WorktreeCreate,
    Settings,
    Logs,
    Help,
    Confirm,
    Error,
    Profiles,
    Environment,
    AISettingsWizard,
    CloneWizard,
    MigrationDialog,
}
```

**追加方法**: `GitView` バリアントを追加し、対応する状態管理を Model に追加

### 1.2 Message enum

**ファイル**: `crates/gwt-cli/src/tui/app.rs` (行546-598)

主要なメッセージ:
- `NavigateTo(Screen)`: 画面遷移
- `NavigateBack`: 前の画面に戻る
- `Char(char)`: キー入力
- `SelectNext` / `SelectPrev`: 上下移動
- `Space`: 展開/折りたたみ
- `Enter`: 決定/リンク開く
- `Tick`: 定期更新
- `RefreshData`: データ再取得

### 1.3 画面遷移パターン

```rust
// ブランチ一覧 → GitView
Message::Char('v') → Message::NavigateTo(Screen::GitView)

// GitView → ブランチ一覧
Message::Char('v') | Message::NavigateBack → screen_stack.pop()
```

スタック形式で前の画面を記憶（`screen_stack: Vec<Screen>`）

### 1.4 render_details_panel の内容

**ファイル**: `crates/gwt-cli/src/tui/screens/branch_list.rs` (行2100-2176)

現在の表示内容:
1. ブランチ名（BranchSummary）
2. ワークツリーパス
3. ローディング状態（スピナー）
4. コミット履歴（最新5件）
5. Diffスタティスティクス
6. メタ情報（ahead/behind, timestamp）
7. セッションサマリー
8. PRリンク（LinkRegion）

**移行対象**: ヘッダー部分（ブランチ名、PR情報、ahead/behind）

### 1.5 BranchItem 構造体

**ファイル**: `crates/gwt-cli/src/tui/screens/branch_list.rs` (行143-172)

```rust
pub struct BranchItem {
    pub name: String,
    pub branch_type: BranchType,
    pub is_current: bool,
    pub has_worktree: bool,
    pub worktree_path: Option<String>,
    pub worktree_status: WorktreeStatus,
    pub has_changes: bool,
    pub has_unpushed: bool,
    pub divergence: DivergenceStatus,
    pub has_remote_counterpart: bool,
    pub remote_name: Option<String>,
    pub safe_to_cleanup: Option<bool>,
    pub safety_status: SafetyStatus,
    pub is_unmerged: bool,
    pub last_commit_timestamp: Option<i64>,
    pub last_tool_usage: Option<String>,
    pub last_tool_id: Option<String>,
    pub last_session_id: Option<String>,
    pub is_selected: bool,
    pub pr_title: Option<String>,
    pub pr_number: Option<u64>,
    pub pr_url: Option<String>,
    pub is_gone: bool,
}
```

**GitView実装で重要なフィールド**:
- `worktree_path`: ファイル一覧取得のためのパス
- `pr_url`: ヘッダーのPRリンク
- `divergence`: ahead/behind情報
- `name`: ブランチ名

### 1.6 バックグラウンドデータ取得パターン

**3段階パターン**:

1. **prepare**: キャッシュ確認、リクエスト生成
   ```rust
   fn prepare_branch_summary(&mut self, repo_root: &Path) -> Option<BranchSummaryRequest>
   ```

2. **build**: 別スレッドでgit情報取得
   ```rust
   fn build_branch_summary(repo_root: &Path, branch: &BranchItem) -> BranchSummary
   ```

3. **apply**: 結果を状態に反映
   ```rust
   fn apply_branch_summary_update(&mut self, update: BranchSummaryUpdate)
   ```

### 1.7 マウスイベント処理

**ファイル**: `crates/gwt-cli/src/tui/app.rs` (行2449-2850)

```rust
fn handle_branch_list_mouse(&mut self, mouse: MouseEvent) {
    // ダブルクリック検出
    if is_double_click {
        if let Some(url) = clicked_url {
            self.open_url(&url);  // リンク開く
        }
    } else {
        // シングルクリック: 選択状態更新
    }
}
```

**LinkRegion**: 位置情報とURLを保持、`link_at_point()` で該当URL検出

## 2. 技術的決定

### 2.1 画面追加

```rust
pub enum Screen {
    // ... 既存
    GitView,  // 新規追加
}
```

### 2.2 状態管理

```rust
// Model に追加
pub git_view: GitViewState,
pub git_view_cache: GitViewCache,
```

### 2.3 キャッシュ戦略

```rust
pub struct GitViewCache {
    data: HashMap<String, GitViewData>,  // ブランチ名 → git情報
}
```

- ブランチ一覧表示時にバックグラウンドで全ブランチを事前取得
- `r`キー（リロード）で全キャッシュ無効化

### 2.4 git情報取得

gwt-core の Repository 経由:
- `git status --porcelain`: ファイル一覧
- `git diff`: 差分内容
- `git log -n 5`: コミット履歴
- `git show`: コミット詳細

## 3. 制約と依存関係

### 3.1 アーキテクチャ制約

- Elmアーキテクチャ準拠（update/view分離）
- Message を介した状態更新
- screen_stack による画面遷移管理

### 3.2 UI制約

- ASCIIアイコンのみ（絵文字不可）
- ratatui のレイアウトシステム使用
- 垂直スタック構成（Layout::vertical）

### 3.3 依存関係

- `BranchItem` から対象ブランチ情報取得
- `BranchListState` から選択状態取得
- `gwt-core::git::Repository` からgitコマンド実行

## 4. 結論

既存のコードパターンに従い、以下の方針で実装:

1. Screen enum に `GitView` 追加
2. `GitViewState` と `GitViewCache` を Model に追加
3. `handle_key_event` で `v` キー処理追加
4. `render_gitview` 関数で画面描画
5. `handle_gitview_mouse` でマウスイベント処理
6. バックグラウンドキャッシュは既存の prepare/build/apply パターン踏襲
