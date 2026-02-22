# 機能仕様: Worktree 作成時に upstream tracking を自動設定

**仕様ID**: `SPEC-b3f1a4e2`
**作成日**: 2026-02-22
**更新日**: 2026-02-22
**ステータス**: ドラフト
**カテゴリ**: Core

## 背景

- gwt で新規ブランチの worktree を作成すると、upstream tracking が設定されない
- そのため初回 push 時に `fatal: no upstream configured for branch` が発生し、手動で `git push -u origin <branch>` を実行する必要がある
- 原因: `Branch::create()` は `git branch <name> <base>` を実行するだけで、`branch.<name>.remote` / `branch.<name>.merge` を設定しない

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - 新規ブランチで worktree 作成後 push 成功 (優先度: P0)

ユーザーとして、新規ブランチで worktree を作成した後、`git push` が upstream 設定済みで成功するようにしたい。

**独立したテスト**: worktree 作成後に `git config branch.<name>.remote` と `branch.<name>.merge` が正しく設定されていることを検証する。

**受け入れシナリオ**:

1. **前提条件** リモート "origin" が存在するリポジトリ、**操作** `create_new_branch("feature/test", None)` を実行、**期待結果** `branch.feature/test.remote = origin` かつ `branch.feature/test.merge = refs/heads/feature/test` が設定されている
2. **前提条件** リモート "origin" が存在するリポジトリ、**操作** `create_for_branch("feature/existing")` を実行、**期待結果** `branch.feature/existing.remote` と `branch.feature/existing.merge` が設定されている

---

### ユーザーストーリー 2 - リモートなしリポジトリでエラーにならない (優先度: P1)

ユーザーとして、リモートが存在しないリポジトリでも worktree 作成がエラーにならずに成功するようにしたい。

**独立したテスト**: リモートなしリポジトリで worktree 作成後、upstream config が設定されず、エラーも発生しないことを検証する。

**受け入れシナリオ**:

1. **前提条件** リモートなしリポジトリ、**操作** `create_new_branch("feature/test", None)` を実行、**期待結果** worktree 作成成功、upstream config は未設定

---

### ユーザーストーリー 3 - remote-first パスでも upstream が設定される (優先度: P1)

ユーザーとして、remote-first パス（`create_new_worktree_remote_first`）でも upstream が自動設定されるようにしたい。

**独立したテスト**: remote-first パスで worktree 作成後、upstream config が "origin" に設定されていることを検証する。

**受け入れシナリオ**:

1. **前提条件** GitHub 連携リポジトリ、**操作** remote-first で worktree 作成、**期待結果** `branch.<name>.remote = origin` かつ `branch.<name>.merge = refs/heads/<name>` が設定されている

## エッジケース

- リモートなしリポジトリ: upstream 設定をスキップし、エラーにしない
- 複数リモート: "origin" を優先、なければ最初のリモートを使用
- upstream 設定の `git config` 失敗: non-fatal として warn ログのみ出力

## 要件 *(必須)*

### 機能要件

- **FR-001**: `Branch::set_upstream_config(repo_path, branch_name, remote)` — `git config` で `branch.<name>.remote` と `branch.<name>.merge` を設定する
- **FR-002**: `Remote::default_name(repo_path)` — デフォルトリモート名を取得する（"origin" 優先、なければ最初のリモート、リモートなしは `None`）
- **FR-003**: `WorktreeManager::create_new_branch()` で worktree 作成後に upstream を自動設定する
- **FR-004**: `WorktreeManager::create_for_branch()` で worktree 作成後に upstream を自動設定する
- **FR-005**: `create_new_worktree_remote_first()` で worktree 作成後に upstream を自動設定する
- **FR-006**: upstream 設定失敗は non-fatal（warn ログのみ）

### 非機能要件

- **NFR-001**: ネットワーク不要（`git config` のみ使用、オフラインでも安全）
- **NFR-002**: リモートブランチが未存在でも設定可能（config 書き込みのみ）

## 制約と仮定

- `git config` コマンドが利用可能であること（git がインストール済み）
- リモート名の取得には既存の `Remote::list()` を使用する

## 成功基準 *(必須)*

- **SC-001**: リモートありリポジトリで `create_new_branch` 後に `git config branch.<name>.remote` と `branch.<name>.merge` が設定されている
- **SC-002**: リモートなしリポジトリで `create_new_branch` 後にエラーが発生しない
- **SC-003**: `create_for_branch` 後に upstream が設定されている
- **SC-004**: upstream 設定の失敗が worktree 作成自体を阻害しない
