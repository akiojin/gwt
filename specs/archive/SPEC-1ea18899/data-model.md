# データモデル設計: GitView画面

**仕様ID**: `SPEC-1ea18899` | **日付**: 2026-02-02

## 概要

GitView画面で使用するデータ構造の定義。

## エンティティ

### GitViewState

GitView画面の状態を管理する構造体。

```rust
pub struct GitViewState {
    /// 対象ブランチ名
    pub branch_name: String,

    /// 対象ブランチのワークツリーパス（None = ワークツリーなし）
    pub worktree_path: Option<PathBuf>,

    /// PR情報
    pub pr_url: Option<String>,
    pub pr_title: Option<String>,

    /// Divergence情報
    pub divergence: DivergenceStatus,

    /// ファイル一覧
    pub files: Vec<FileEntry>,

    /// 表示中のファイル数（Show more用）
    pub visible_file_count: usize,

    /// コミット一覧（最新5件）
    pub commits: Vec<CommitEntry>,

    /// 現在の選択位置（統一インデックス）
    pub selected_index: usize,

    /// 展開状態（インデックス → 展開フラグ）
    pub expanded: HashSet<usize>,

    /// PRリンクの領域（マウスクリック用）
    pub pr_link_region: Option<LinkRegion>,

    /// ローディング状態
    pub is_loading: bool,
}
```

**属性**:
| フィールド | 型 | 説明 |
|-----------|---|------|
| branch_name | String | 対象ブランチ名 |
| worktree_path | Option\<PathBuf\> | ワークツリーパス（なければNone） |
| pr_url | Option\<String\> | PRのURL |
| pr_title | Option\<String\> | PRタイトル |
| divergence | DivergenceStatus | ahead/behind情報 |
| files | Vec\<FileEntry\> | 変更ファイル一覧 |
| visible_file_count | usize | 表示中のファイル数（初期20） |
| commits | Vec\<CommitEntry\> | コミット履歴（最新5件） |
| selected_index | usize | 選択位置（0=PRリンク, 1~=Files, ...=Commits） |
| expanded | HashSet\<usize\> | 展開中のアイテムのインデックス |
| pr_link_region | Option\<LinkRegion\> | PRリンクのクリック領域 |
| is_loading | bool | データ読み込み中フラグ |

### GitViewCache

全ブランチのgit情報をキャッシュする構造体。

```rust
pub struct GitViewCache {
    /// ブランチ名 → キャッシュデータ
    data: HashMap<String, GitViewData>,
}
```

### GitViewData

1ブランチ分のキャッシュデータ。

```rust
pub struct GitViewData {
    /// ファイル一覧
    pub files: Vec<FileEntry>,

    /// コミット一覧
    pub commits: Vec<CommitEntry>,

    /// キャッシュ作成時刻
    pub cached_at: Instant,
}
```

### FileEntry

変更ファイルの情報。

```rust
pub struct FileEntry {
    /// ファイルパス
    pub path: String,

    /// ステータス
    pub status: FileStatus,

    /// バイナリファイルかどうか
    pub is_binary: bool,

    /// サイズ変化（バイナリの場合）
    pub size_change: Option<i64>,

    /// diff内容（展開時に遅延取得）
    pub diff: Option<String>,

    /// diff行数（省略判定用）
    pub diff_line_count: usize,
}

pub enum FileStatus {
    Staged,     // [S]
    Unstaged,   // [U]
    Untracked,  // [?]
}
```

**属性**:
| フィールド | 型 | 説明 |
|-----------|---|------|
| path | String | ファイルパス |
| status | FileStatus | staged/unstaged/untracked |
| is_binary | bool | バイナリファイルフラグ |
| size_change | Option\<i64\> | サイズ変化（バイト） |
| diff | Option\<String\> | diff内容（遅延取得） |
| diff_line_count | usize | diff総行数 |

### CommitEntry

コミット情報。

```rust
pub struct CommitEntry {
    /// コミットハッシュ（短縮形）
    pub hash: String,

    /// コミットメッセージ（1行目）
    pub subject: String,

    /// コミットメッセージ（全文、展開時用）
    pub body: Option<String>,

    /// 著者名
    pub author: String,

    /// コミット日時
    pub date: DateTime<Utc>,

    /// 変更ファイル一覧（展開時用）
    pub changed_files: Vec<String>,
}
```

**属性**:
| フィールド | 型 | 説明 |
|-----------|---|------|
| hash | String | コミットハッシュ（7文字） |
| subject | String | コミットメッセージ1行目 |
| body | Option\<String\> | メッセージ全文 |
| author | String | 著者名 |
| date | DateTime\<Utc\> | コミット日時 |
| changed_files | Vec\<String\> | 変更ファイル一覧 |

## 関係図

```text
GitViewCache (1) ──contains──> (*) GitViewData
                                      │
                                      ├── (*) FileEntry
                                      │
                                      └── (*) CommitEntry

GitViewState (1) ──references──> GitViewData (via branch_name)
```

## インデックス計算

GitView画面では、ヘッダー・Files・Commitsを統一インデックスで管理:

```text
インデックス 0: PRリンク（存在する場合）
インデックス 1 ~ N: Filesセクションのアイテム
  - 通常ファイル: 1アイテム
  - 展開中ファイル: 1 + min(diff_lines, 50) アイテム
  - Show more: 1アイテム（残りがある場合）
インデックス N+1 ~ M: Commitsセクションのアイテム
  - 通常コミット: 1アイテム
  - 展開中コミット: 1 + changed_files.len() アイテム
```

## 検証ルール

1. **FileEntry.path**: 空文字列不可
2. **CommitEntry.hash**: 7文字以上
3. **GitViewState.visible_file_count**: 0 ≤ value ≤ files.len()
4. **GitViewState.selected_index**: 有効なインデックス範囲内
