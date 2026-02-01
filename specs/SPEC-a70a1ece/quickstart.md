# クイックスタート: bareリポジトリ対応とヘッダーブランチ表示

**仕様ID**: `SPEC-a70a1ece`
**作成日**: 2026-02-01

## 1. 開発環境セットアップ

### 1.1 ビルドと実行

```bash
# ビルド
cargo build --release

# テスト実行
cargo test

# Lint
cargo clippy --all-targets --all-features -- -D warnings

# フォーマット
cargo fmt
```

### 1.2 テスト用bareリポジトリの作成

```bash
# テスト用ディレクトリを作成
mkdir -p /tmp/gwt-test-bare
cd /tmp/gwt-test-bare

# 空のbareリポジトリを作成
git init --bare test-repo.git

# または、既存リポジトリからclone
git clone --bare https://github.com/user/repo.git test-repo.git
```

### 1.3 テスト用worktreeの作成

```bash
cd /tmp/gwt-test-bare

# bareリポジトリからworktreeを作成
git -C test-repo.git worktree add ../main main
git -C test-repo.git worktree add ../feature-x feature-x
```

## 2. 開発ワークフロー

### 2.1 ヘッダー表示変更のテスト

```bash
# 通常リポジトリでテスト
cd /path/to/normal-repo
cargo run

# bareリポジトリでテスト
cd /tmp/gwt-test-bare/test-repo.git
cargo run

# worktree内でテスト
cd /tmp/gwt-test-bare/main
cargo run
```

**期待される表示**:

| 起動場所 | ヘッダー表示 |
|----------|-------------|
| 通常リポジトリ | `Working Directory: /path [branch]` |
| bareリポジトリ | `Working Directory: /path/repo.git [bare]` |
| worktree（通常） | `Working Directory: /path [branch]` |
| worktree（bare方式） | `Working Directory: /path [branch] (repo.git)` |

### 2.2 (current)ラベル削除の確認

```bash
# gwtを起動してブランチリストを表示
cargo run

# ブランチリストに "(current)" が表示されないことを確認
```

### 2.3 空ディレクトリでのテスト

```bash
# 空ディレクトリを作成
mkdir -p /tmp/gwt-empty-test
cd /tmp/gwt-empty-test

# gwtを起動（cloneウィザードが表示されるはず）
cargo run
```

### 2.4 CLIオプションのテスト

```bash
# gwt init コマンドのテスト
cd /tmp/gwt-cli-test
cargo run -- init https://github.com/user/repo.git

# shallow cloneがデフォルトで実行されることを確認
ls -la  # repo.git が作成されているはず
```

## 3. 統合テストの実行

### 3.1 テストファイルの場所

```text
crates/gwt-core/src/
├── git/
│   └── repository_test.rs  # RepoType検出テスト
└── worktree/
    └── manager_test.rs     # WorktreeLocation分岐テスト

crates/gwt-cli/src/
└── tui/
    ├── app_test.rs         # ヘッダー表示テスト
    └── screens/
        └── branch_list_test.rs  # (current)削除テスト
```

### 3.2 テスト実行

```bash
# 全テスト
cargo test

# 特定のテストのみ
cargo test test_repo_type_detection
cargo test test_header_display
cargo test test_current_label_removed
```

## 4. よくある操作

### 4.1 bareリポジトリの検出確認

```bash
# gitコマンドで直接確認
git rev-parse --is-bare-repository
# true または false

# worktree内かどうか確認
git rev-parse --is-inside-work-tree
# true または false
```

### 4.2 worktree一覧の確認

```bash
# gitコマンドで確認
git worktree list
```

### 4.3 設定ファイルの確認

```bash
# .gwt/config.json の内容確認
cat .gwt/config.json
```

## 5. トラブルシューティング

### 5.1 bareリポジトリが検出されない

**症状**: bareリポジトリ内でgwtを起動しても `[bare]` が表示されない

**確認手順**:

```bash
# 1. gitコマンドで確認
git rev-parse --is-bare-repository

# 2. ディレクトリ構造を確認
ls -la
# HEAD, config, objects/, refs/ などが直接存在するはず
```

### 5.2 worktreeが認識されない

**症状**: worktree内でgwtを起動しても親bareが検出されない

**確認手順**:

```bash
# 1. .gitファイルの内容を確認
cat .git
# gitdir: /path/to/bare.git/worktrees/branch-name

# 2. 親bareリポジトリの存在確認
ls -la $(cat .git | sed 's/gitdir: //' | sed 's|/worktrees/.*||')
```

### 5.3 cloneが失敗する

**症状**: URL入力後にcloneが失敗

**確認手順**:

```bash
# 1. gitコマンドで直接テスト
git clone --bare <url> test.git

# 2. 認証情報の確認
git credential fill <<< "protocol=https
host=github.com"
```

### 5.4 worktree作成が失敗する

**症状**: ブランチ選択後にworktree作成が失敗

**確認手順**:

```bash
# 1. 書き込み権限の確認
touch ../test-file && rm ../test-file

# 2. 既存worktreeの確認
git worktree list

# 3. 手動で作成を試みる
git worktree add ../branch-name branch-name
```

## 6. 関連ドキュメント

- [spec.md](./spec.md) - 機能仕様
- [plan.md](./plan.md) - 実装計画
- [data-model.md](./data-model.md) - データモデル設計
- [research.md](./research.md) - 調査結果
