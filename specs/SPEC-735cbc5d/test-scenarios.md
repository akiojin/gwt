# テストシナリオ: GitView in Session Summary

**SPEC ID**: `SPEC-735cbc5d`

## バックエンドテスト（Rust）

### 1. diff.rs - ブランチ差分ファイル一覧

#### T-DIFF-001: 基本的なブランチ差分ファイル取得

```text
Given: テスト用リポジトリに main ブランチと feature ブランチがあり、
       feature ブランチで 3 ファイルを変更済み
When:  get_branch_diff_files(repo, "feature", "main") を呼び出す
Then:  3 件の FileChange が返却され、各 FileChange に path, kind,
       additions, deletions が設定されている
```

#### T-DIFF-002: ファイル追加の検出

```text
Given: feature ブランチで新規ファイル "new.rs" を追加
When:  get_branch_diff_files() を呼び出す
Then:  FileChange { path: "new.rs", kind: Added, additions > 0,
       deletions: 0, is_binary: false } が含まれる
```

#### T-DIFF-003: ファイル削除の検出

```text
Given: feature ブランチでファイル "old.rs" を削除
When:  get_branch_diff_files() を呼び出す
Then:  FileChange { path: "old.rs", kind: Deleted, additions: 0,
       deletions > 0, is_binary: false } が含まれる
```

#### T-DIFF-004: ファイル変更の検出

```text
Given: feature ブランチでファイル "lib.rs" を編集（3 行追加、1 行削除）
When:  get_branch_diff_files() を呼び出す
Then:  FileChange { path: "lib.rs", kind: Modified, additions: 3,
       deletions: 1, is_binary: false } が含まれる
```

#### T-DIFF-005: バイナリファイルの検出

```text
Given: feature ブランチでバイナリファイル "image.png" を追加
When:  get_branch_diff_files() を呼び出す
Then:  FileChange { path: "image.png", kind: Added,
       is_binary: true } が含まれる
```

#### T-DIFF-006: 変更なしブランチ

```text
Given: feature ブランチが main と同一コミット
When:  get_branch_diff_files() を呼び出す
Then:  空の Vec<FileChange> が返却される
```

### 2. diff.rs - 個別ファイル diff

#### T-DIFF-010: 基本的な diff 取得

```text
Given: feature ブランチで "lib.rs" に変更あり
When:  get_file_diff(repo, "feature", "main", "lib.rs") を呼び出す
Then:  FileDiff { content: (unified diff テキスト), truncated: false }
       が返却される
```

#### T-DIFF-011: 1000 行超 diff の切り詰め

```text
Given: feature ブランチで "large.rs" に 2000 行の変更あり
When:  get_file_diff() を呼び出す
Then:  FileDiff { content: (1000 行分のみ), truncated: true }
       が返却される
```

#### T-DIFF-012: バイナリファイルの diff

```text
Given: feature ブランチで "image.png" を変更
When:  get_file_diff() を呼び出す
Then:  FileDiff { content: "Binary file changed", truncated: false }
       が返却される（またはエラー）
```

### 3. diff.rs - Working tree 状態

#### T-DIFF-020: Staged ファイルの取得

```text
Given: "staged.rs" が git add 済み（staged）
When:  get_working_tree_status() を呼び出す
Then:  WorkingTreeEntry { path: "staged.rs", is_staged: true }
       が含まれる
```

#### T-DIFF-021: Unstaged ファイルの取得

```text
Given: "modified.rs" が変更済みだが未 staged
When:  get_working_tree_status() を呼び出す
Then:  WorkingTreeEntry { path: "modified.rs", is_staged: false }
       が含まれる
```

#### T-DIFF-022: 変更なしの Working tree

```text
Given: Working tree がクリーン
When:  get_working_tree_status() を呼び出す
Then:  空の Vec<WorkingTreeEntry> が返却される
```

### 4. diff.rs - コミット一覧

#### T-DIFF-030: 基本的なコミット取得

```text
Given: feature ブランチに main から 5 コミット先行
When:  get_branch_commits(repo, "feature", "main", 0, 20) を呼び出す
Then:  5 件の CommitEntry が返却され、各エントリに sha, message,
       timestamp, author が設定されている
```

#### T-DIFF-031: ページネーション（offset/limit）

```text
Given: feature ブランチに main から 30 コミット先行
When:  get_branch_commits(repo, "feature", "main", 0, 20) を呼び出す
Then:  20 件の CommitEntry が返却される
When:  get_branch_commits(repo, "feature", "main", 20, 20) を呼び出す
Then:  10 件の CommitEntry が返却される
```

#### T-DIFF-032: コミット 0 件

```text
Given: feature ブランチが main と同一コミット
When:  get_branch_commits() を呼び出す
Then:  空の Vec<CommitEntry> が返却される
```

### 5. diff.rs - 基準ブランチ検出

#### T-DIFF-040: upstream 設定あり

```text
Given: feature ブランチの upstream が "origin/develop" に設定
When:  detect_base_branch(repo, "feature") を呼び出す
Then:  "develop"（または "origin/develop"）が返却される
```

#### T-DIFF-041: upstream 未設定、main 存在

```text
Given: feature ブランチの upstream が未設定、"main" ブランチが存在
When:  detect_base_branch() を呼び出す
Then:  "main" が返却される
```

#### T-DIFF-042: 基準ブランチ候補一覧

```text
Given: リポジトリに main, develop ブランチが存在
When:  list_base_branch_candidates() を呼び出す
Then:  ["main", "develop"] が返却される
```

### 6. diff.rs - サマリー

#### T-DIFF-050: サマリー集約

```text
Given: feature ブランチに 5 ファイル変更、3 コミット、1 stash
When:  get_git_change_summary() を呼び出す
Then:  GitChangeSummary { file_count: 5, commit_count: 3,
       stash_count: 1, base_branch: "main" } が返却される
```

### 7. stash.rs - Stash 一覧

#### T-STASH-001: 基本的な stash 取得

```text
Given: stash に 2 エントリあり
When:  get_stash_list() を呼び出す
Then:  2 件の StashEntry が返却され、各エントリに index, message,
       file_count が設定されている
```

#### T-STASH-002: stash 0 件

```text
Given: stash が空
When:  get_stash_list() を呼び出す
Then:  空の Vec<StashEntry> が返却される
```

## フロントエンドテスト（Svelte - 手動検証項目）

### UI 表示テスト

#### T-UI-001: Git セクション折りたたみ/展開

```text
Given: Session Summary タブが表示されている
When:  Git セクションヘッダーをクリック
Then:  セクションが展開され、Changes タブが表示される
When:  再度ヘッダーをクリック
Then:  セクションが折りたたまれる
```

#### T-UI-002: ヘッダーサマリー表示

```text
Given: ブランチに変更あり（5 files, 3 commits, 1 stash）
When:  Session Summary タブを表示
Then:  折りたたまれた Git セクションヘッダーに
       "5 files, 3 commits, 1 stash" が表示される
```

#### T-UI-003: ローディング状態

```text
Given: Git セクションを展開した直後
When:  データ取得中
Then:  スピナー + "Loading git info..." が表示される
When:  データ取得完了
Then:  スピナーが消え、コンテンツが表示される
```

#### T-UI-004: リフレッシュボタン

```text
Given: Git セクションが展開されている
When:  リフレッシュボタンをクリック
Then:  スピナーが表示され、全データが再取得される
```

#### T-UI-005: ファイルツリーと diff 展開

```text
Given: Changes タブが表示されている
Then:  ファイルがディレクトリツリー形式で全展開表示される
       各ファイルに 5 ブロック統計バーと "+N -N" 数値が表示される
When:  ファイルをクリック
Then:  unified diff が展開表示される（緑: 追加、赤: 削除）
When:  再度クリック
Then:  diff が折りたたまれる
```

#### T-UI-006: Committed/Uncommitted フィルター

```text
Given: Changes タブが表示されている（Committed がデフォルト）
When:  "Uncommitted" をクリック
Then:  Working tree 変更が Staged / Unstaged サブセクションで表示される
When:  "Committed" をクリック
Then:  ブランチ差分ファイルに戻る
```

#### T-UI-007: コミットリストと Show more

```text
Given: Commits タブを選択、30 件のコミットあり
Then:  最新 20 件が表示され、"Show more" ボタンが見える
When:  "Show more" をクリック
Then:  残り 10 件が追加表示され、"Show more" ボタンが消える
```

#### T-UI-008: コミット日時の相対/絶対表示

```text
Given: Commits タブにコミットが表示されている
Then:  日時は "2 hours ago" 等の相対表示
When:  日時にマウスを hover
Then:  "2026-02-10 14:30" 等の絶対日時がツールチップ表示される
```

#### T-UI-009: Stash タブの条件付き表示

```text
Given: stash が 2 件ある
Then:  Stash タブが表示され、2 件の stash が見える
Given: stash が 0 件
Then:  Stash タブ自体が非表示
```

#### T-UI-010: 基準ブランチドロップダウン

```text
Given: Git セクションが展開されている
When:  基準ブランチドロップダウンを開く
Then:  main / develop（存在するもの）が選択肢として表示される
When:  "develop" を選択
Then:  Changes と Commits が develop との差分で再表示される
```

### エッジケーステスト

#### T-UI-020: バイナリファイル

```text
Given: Changes タブにバイナリファイルがある
Then:  "Binary file changed" と表示される
When:  バイナリファイルをクリック
Then:  diff は展開されない（展開不可）
```

#### T-UI-021: 大規模 diff

```text
Given: Changes タブに 1000 行超のファイルがある
When:  ファイルをクリックして diff 展開
Then:  1000 行で切り詰められ "Too large to display" が末尾に表示される
```

#### T-UI-022: upstream 未設定

```text
Given: ブランチに upstream が設定されていない
When:  Git セクションを表示
Then:  main にフォールバックして差分が表示される
```

#### T-UI-023: 非 Git リポジトリ

```text
Given: Git リポジトリでないプロジェクトを開いている
When:  Session Summary タブを表示
Then:  Git セクションが表示されない
```
