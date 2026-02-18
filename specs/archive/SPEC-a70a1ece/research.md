# 調査結果: bareリポジトリ対応とヘッダーブランチ表示

**仕様ID**: `SPEC-a70a1ece`
**調査日**: 2026-02-01

## 1. 既存コードベース分析

### 1.1 ヘッダー表示（Working Directory）

**場所**: `crates/gwt-cli/src/tui/app.rs:5354-5367`

```rust
// Line 1: Working Directory
let working_dir_line = Line::from(vec![
    Span::raw(" "),
    Span::styled("Working Directory: ", Style::default().fg(Color::DarkGray)),
    Span::raw(working_dir),
]);
frame.render_widget(Paragraph::new(working_dir_line), inner_chunks[0]);
```

**変更方針**:
- `working_dir` の後に `[branch-name]` を追加
- bareリポジトリの場合は `[bare]` を表示
- bare方式worktreeの場合は `[branch-name] (repo.git)` を表示

### 1.2 (current)ラベル表示

**場所**: `crates/gwt-cli/src/tui/screens/branch_list.rs:1855`

```rust
let current_label = if branch.is_current { " (current)" } else { "" };
```

**変更方針**:
- この行を削除（`current_label` を空文字列に固定）
- `is_current` フラグ自体は残す（ソート等で使用）

### 1.3 worktree管理

**場所**: `crates/gwt-core/src/worktree/manager.rs`

主要メソッド:
- `WorktreeManager::new(repo_root: PathBuf)` - 初期化
- `WorktreeManager::list()` - 全worktree一覧
- `WorktreeManager::create_for_branch()` - 既存ブランチでworktree作成
- `WorktreeManager::create_new_branch()` - 新ブランチでworktree作成

**変更方針**:
- `WorktreeLocation` enumを追加して配置方式を分岐
- bare方式の場合は親ディレクトリにworktreeを作成

### 1.4 gitコマンド実行パターン

**場所**: `crates/gwt-core/src/git/branch.rs`

```rust
use std::process::Command;

let output = Command::new("git")
    .args(["for-each-ref", "--format=...", "refs/heads/"])
    .current_dir(repo_path)
    .output()?;

if !output.status.success() {
    return Err(anyhow!("git command failed"));
}
```

**パターン**:
- `std::process::Command::new("git")`
- `.args([...])` で引数指定
- `.current_dir(path)` で作業ディレクトリ指定
- `.output()` で実行・結果取得
- `status.success()` でエラーチェック

### 1.5 ウィザードUIパターン

**場所**: `crates/gwt-cli/src/tui/screens/worktree_create.rs`

構成:
- `WorktreeCreateState` - 状態管理構造体
- `WorktreeCreateStep` - ステップenum
- `render_worktree_create()` - メインレンダリング関数
- `render_*_step()` - 各ステップのレンダリング

**変更方針**:
- 同様のパターンで `CloneWizardState` と `CloneWizardStep` を作成
- 既存のウィザードUIコンポーネントを再利用

## 2. 技術的決定

### 2.1 bareリポジトリ検出

**方法**: `git rev-parse --is-bare-repository`

```rust
fn is_bare_repository(path: &Path) -> bool {
    let output = Command::new("git")
        .args(["rev-parse", "--is-bare-repository"])
        .current_dir(path)
        .output();

    match output {
        Ok(o) if o.status.success() => {
            String::from_utf8_lossy(&o.stdout).trim() == "true"
        }
        _ => false,
    }
}
```

### 2.2 リポジトリ種別検出

**新規enum**: `RepoType`

```rust
pub enum RepoType {
    Normal,     // 通常リポジトリ（.git/がディレクトリ）
    Bare,       // bareリポジトリ
    Worktree,   // worktree内
    Empty,      // 空ディレクトリ
    NonRepo,    // gitリポジトリでない（空でない）
}
```

**検出ロジック**:

```rust
fn detect_repo_type(path: &Path) -> RepoType {
    // 1. ディレクトリが空か確認
    if is_empty_dir(path) {
        return RepoType::Empty;
    }

    // 2. gitリポジトリか確認
    if !is_git_repo(path) {
        return RepoType::NonRepo;
    }

    // 3. bareリポジトリか確認
    if is_bare_repository(path) {
        return RepoType::Bare;
    }

    // 4. worktree内か確認
    if is_inside_worktree(path) {
        return RepoType::Worktree;
    }

    RepoType::Normal
}
```

### 2.3 worktree配置方式

**新規enum**: `WorktreeLocation`

```rust
pub enum WorktreeLocation {
    Subdir,   // .worktrees/ 配下（従来方式）
    Sibling,  // bare同階層（新方式）
}
```

**決定ロジック**:
- bareリポジトリ → `Sibling`
- 通常リポジトリ → `Subdir`

### 2.4 ヘッダーコンテキスト

**新規構造体**: `HeaderContext`

```rust
pub struct HeaderContext {
    pub working_dir: PathBuf,
    pub branch_name: Option<String>,
    pub repo_type: RepoType,
    pub bare_name: Option<String>,
}
```

**表示ロジック**:

```rust
fn format_header(ctx: &HeaderContext) -> String {
    let path = ctx.working_dir.display();
    match ctx.repo_type {
        RepoType::Bare => format!("{} [bare]", path),
        RepoType::Worktree if ctx.bare_name.is_some() => {
            format!("{} [{}] ({})", path, ctx.branch_name.unwrap_or_default(), ctx.bare_name.unwrap())
        }
        _ => format!("{} [{}]", path, ctx.branch_name.unwrap_or_default()),
    }
}
```

## 3. 制約と依存関係

### 3.1 後方互換性

- 通常リポジトリ + `.worktrees/` 方式は完全に維持
- `RepoType::Normal` の場合は従来のロジックをそのまま使用
- 既存のテストがすべてパスすることを確認

### 3.2 gitコマンド依存

- `git clone --bare` - bare clone
- `git worktree add` - worktree作成
- `git rev-parse --is-bare-repository` - bare検出
- `git rev-parse --git-dir` - .gitディレクトリ検出
- `git submodule init/update` - submodule初期化

### 3.3 ファイルシステム

- bareリポジトリ: `{project}/{repo-name}.git/`
- worktree: `{project}/{branch-name}/`
- 設定: `{project}/.gwt/`
- スラッシュ含むブランチ: `{project}/feature/branch-name/`

## 4. 調査結論

### 変更対象ファイル

| ファイル | 変更内容 |
|----------|----------|
| `gwt-cli/src/tui/app.rs` | ヘッダー表示変更 |
| `gwt-cli/src/tui/screens/branch_list.rs` | (current)ラベル削除 |
| `gwt-cli/src/main.rs` | `gwt init` CLIオプション追加 |
| `gwt-core/src/git/repository.rs` | `RepoType` 検出ロジック追加 |
| `gwt-core/src/worktree/manager.rs` | `WorktreeLocation` 分岐追加 |

### 新規ファイル

| ファイル | 内容 |
|----------|------|
| `gwt-cli/src/tui/screens/clone_wizard.rs` | bare cloneウィザード |
| `gwt-cli/src/tui/screens/migration_dialog.rs` | マイグレーションダイアログ |
| `gwt-core/src/migration/mod.rs` | マイグレーションモジュール |
| `gwt-core/src/migration/executor.rs` | マイグレーション実行ロジック |
| `gwt-core/src/migration/validator.rs` | マイグレーション前検証 |
| `gwt-core/src/migration/rollback.rs` | ロールバック処理 |

### 未確定事項

なし（clarifyで解消済み）
