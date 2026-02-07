# データモデル設計: bareリポジトリ対応とヘッダーブランチ表示

**仕様ID**: `SPEC-a70a1ece`
**作成日**: 2026-02-01

## 1. 主要エンティティ

### 1.1 RepoType（リポジトリ種別）

```rust
/// gwtが起動されたディレクトリの種別
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoType {
    /// 通常のgitリポジトリ（.git/がディレクトリ）
    Normal,
    /// bareリポジトリ
    Bare,
    /// worktree内（通常またはbare方式）
    Worktree,
    /// 空のディレクトリ（gitリポジトリではない）
    Empty,
    /// gitリポジトリでない（空でないディレクトリ）
    NonRepo,
}
```

**属性**:
- `Normal`: `.git/` ディレクトリが存在し、`is-bare-repository` が false
- `Bare`: `is-bare-repository` が true
- `Worktree`: `.git` がファイルで、親リポジトリを指している
- `Empty`: ディレクトリが空（隠しファイルも含まない）
- `NonRepo`: ファイルが存在するがgitリポジトリではない

### 1.2 WorktreeLocation（worktree配置方式）

```rust
/// worktreeの配置方式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorktreeLocation {
    /// .worktrees/ 配下に配置（従来方式）
    Subdir,
    /// bareリポジトリと同階層に配置（新方式）
    Sibling,
}
```

**決定ルール**:
- `RepoType::Normal` → `WorktreeLocation::Subdir`
- `RepoType::Bare` → `WorktreeLocation::Sibling`
- `RepoType::Worktree` → 親リポジトリの方式を継承

### 1.3 CloneConfig（clone設定）

```rust
/// bare clone時の設定
#[derive(Debug, Clone)]
pub struct CloneConfig {
    /// リモートリポジトリURL
    pub url: String,
    /// shallow cloneを行うか
    pub shallow: bool,
    /// shallow cloneの深さ（shallow=trueの場合のみ有効）
    pub depth: u32,
}

impl Default for CloneConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            shallow: true,  // CLIデフォルト
            depth: 1,
        }
    }
}
```

**検証ルール**:
- `url` は空でない
- `depth` は1以上（shallow=trueの場合）

### 1.4 HeaderContext（ヘッダー表示コンテキスト）

```rust
/// ヘッダー表示に必要な情報
#[derive(Debug, Clone)]
pub struct HeaderContext {
    /// 作業ディレクトリのパス
    pub working_dir: PathBuf,
    /// 現在のブランチ名（bareの場合はNone）
    pub branch_name: Option<String>,
    /// リポジトリ種別
    pub repo_type: RepoType,
    /// bareリポジトリ名（worktree内でbare方式の場合）
    pub bare_name: Option<String>,
}
```

**表示フォーマット**:

| RepoType | bare_name | 表示形式 |
|----------|-----------|----------|
| Normal | - | `Working Directory: /path [branch]` |
| Bare | - | `Working Directory: /path [bare]` |
| Worktree | None | `Working Directory: /path [branch]` |
| Worktree | Some | `Working Directory: /path [branch] (repo.git)` |

### 1.5 BareProjectConfig（bareプロジェクト設定）

```rust
/// bareプロジェクトの設定（.gwt/config.json）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BareProjectConfig {
    /// bareリポジトリのディレクトリ名（例: "my-repo.git"）
    pub bare_repo_name: String,
    /// worktree配置方式
    pub worktree_location: WorktreeLocation,
    /// submodule自動初期化
    pub auto_init_submodules: bool,
}

impl Default for BareProjectConfig {
    fn default() -> Self {
        Self {
            bare_repo_name: String::new(),
            worktree_location: WorktreeLocation::Sibling,
            auto_init_submodules: true,
        }
    }
}
```

### 1.6 MigrationConfig（マイグレーション設定）

```rust
/// マイグレーション実行時の設定
#[derive(Debug, Clone)]
pub struct MigrationConfig {
    /// 元のリポジトリルート
    pub source_root: PathBuf,
    /// 新しいプロジェクトルート（bareの親）
    pub target_root: PathBuf,
    /// bareリポジトリ名（例: "gwt.git"）
    pub bare_repo_name: String,
    /// バックアップディレクトリ
    pub backup_dir: PathBuf,
}
```

### 1.7 MigrationState（マイグレーション状態）

```rust
/// マイグレーションの進行状態
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationState {
    /// 未開始
    NotStarted,
    /// バックアップ中
    BackingUp,
    /// bareリポジトリ作成中
    CreatingBare,
    /// worktree移行中（インデックス付き）
    MigratingWorktree(usize, usize),
    /// クリーンアップ中
    CleaningUp,
    /// 完了
    Completed,
    /// ロールバック中
    RollingBack,
    /// 失敗
    Failed(MigrationError),
}
```

### 1.8 MigrationError（マイグレーションエラー）

```rust
/// マイグレーションエラーの種類
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationError {
    /// ディスク容量不足
    InsufficientDiskSpace { required: u64, available: u64 },
    /// locked worktreeが存在
    LockedWorktree(String),
    /// ネットワークエラー（リトライ回数付き）
    NetworkError { message: String, retries: u8 },
    /// ファイル操作エラー
    FileSystemError(String),
    /// gitコマンドエラー
    GitCommandError(String),
    /// ロールバック失敗
    RollbackFailed(String),
}
```

### 1.9 WorktreeMigrationInfo（worktree移行情報）

```rust
/// 個別worktreeの移行情報
#[derive(Debug, Clone)]
pub struct WorktreeMigrationInfo {
    /// ブランチ名
    pub branch_name: String,
    /// 元のパス
    pub source_path: PathBuf,
    /// 新しいパス
    pub target_path: PathBuf,
    /// dirty状態か（未コミット変更あり）
    pub is_dirty: bool,
    /// submoduleを含むか
    pub has_submodules: bool,
    /// stashが存在するか
    pub has_stash: bool,
    /// git hooksが存在するか
    pub has_hooks: bool,
}
```

## 2. 関係図

```text
┌─────────────────────────────────────────────────────────────┐
│                     プロジェクトディレクトリ                     │
│  /projects/my-project/                                      │
├─────────────────────────────────────────────────────────────┤
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐         │
│  │ .gwt/       │  │ my-repo.git │  │ main/       │         │
│  │ (設定)      │  │ (bare)      │  │ (worktree)  │         │
│  └─────────────┘  └─────────────┘  └─────────────┘         │
│         │               │               │                   │
│         │               │               │                   │
│  BareProjectConfig  RepoType::Bare  RepoType::Worktree     │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

## 3. 状態遷移

### 3.1 gwt起動時の状態遷移

```text
[起動]
   │
   ▼
┌──────────────────┐
│  detect_repo_type │
└────────┬─────────┘
         │
   ┌─────┼─────┬─────────┬──────────┐
   ▼     ▼     ▼         ▼          ▼
Empty  Bare  Normal  Worktree   NonRepo
   │     │     │         │          │
   ▼     │     │         │          ▼
Clone   │     │         │       Warning
Wizard  │     │         │       + Clone
   │    └──┬──┘         │       Wizard
   │       │            │
   ▼       ▼            ▼
Branch   Branch      Branch
List     List        List
```

### 3.2 マイグレーション状態遷移

```text
[.worktrees/方式検出]
         │
         ▼
┌────────────────────┐
│ ダイアログ表示     │
└────────┬───────────┘
         │
    ┌────┴────┐
    ▼         ▼
 続行       拒否
    │         │
    │         ▼
    │     [gwt終了]
    │
    ▼
┌────────────────────┐
│ ディスク容量チェック│
└────────┬───────────┘
         │
    ┌────┴────┐
    ▼         ▼
  OK       不足
    │         │
    │         ▼
    │     [ブロック]
    │
    ▼
┌────────────────────┐
│ Locked WT チェック │
└────────┬───────────┘
         │
    ┌────┴────┐
    ▼         ▼
  OK      Locked
    │         │
    │         ▼
    │     [ブロック]
    │
    ▼
┌────────────────────┐
│ バックアップ作成   │
└────────┬───────────┘
         │
         ▼
┌────────────────────┐
│ bareリポジトリ作成 │
└────────┬───────────┘
         │
         ▼
┌────────────────────┐     エラー     ┌──────────────┐
│ worktree移行       │───────────────→│ ロールバック │
│ (順次処理)         │                └──────────────┘
└────────┬───────────┘
         │
         ▼
┌────────────────────┐
│ 元ディレクトリ削除 │
└────────┬───────────┘
         │
         ▼
┌────────────────────┐
│ 完了通知           │
└────────────────────┘
```

### 3.3 worktree作成時の状態遷移

```text
[worktree作成開始]
         │
         ▼
┌────────────────────┐
│ WorktreeLocation   │
│ 判定               │
└────────┬───────────┘
         │
    ┌────┴────┐
    ▼         ▼
 Subdir    Sibling
    │         │
    ▼         ▼
.worktrees/  ../
{branch}/   {branch}/
    │         │
    └────┬────┘
         ▼
┌────────────────────┐
│ git worktree add   │
└────────┬───────────┘
         │
         ▼
┌────────────────────┐
│ submodule init     │
│ (auto_init=true)   │
└────────────────────┘
```

## 4. 検証ルール

### 4.1 CloneConfig検証

- `url` が空の場合 → エラー「URL is required」
- `url` が不正な形式 → gitに委譲（gitコマンドがエラーを返す）
- `depth < 1` の場合 → 1に補正

### 4.2 ブランチ名検証

- スラッシュを含む場合 → サブディレクトリ構造で作成
- 既にworktreeが存在する場合 → 確認ダイアログを表示

### 4.3 ディレクトリ権限検証

- 書き込み権限がない場合 → エラー「Permission denied」
- 親ディレクトリが存在しない場合 → 作成を試みる

## 5. 永続化

### 5.1 設定ファイル

**場所**: `{project}/.gwt/config.json`

```json
{
  "bare_repo_name": "my-repo.git",
  "worktree_location": "sibling",
  "auto_init_submodules": true
}
```

### 5.2 既存設定との互換性

- 既存の `.gwt/` 設定は維持
- `bare_repo_name` が未設定の場合は従来方式（Subdir）と判断
