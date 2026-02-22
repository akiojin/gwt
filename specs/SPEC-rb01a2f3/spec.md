# 機能仕様: リモートブランチ存在時のIssueブランチ検出修正

**仕様ID**: `SPEC-rb01a2f3`
**作成日**: 2026-02-22
**更新日**: 2026-02-22
**ステータス**: 承認済み
**カテゴリ**: Core / Git Issue
**依存仕様**:

- SPEC-e4798383（GitHub Issue operations）

**入力**: ユーザー説明: "Issueから「Work on this」でブランチ作成時、リモートにすでに同名ブランチが存在するとエラーになる"

## 背景

- `find_branch_for_issue()` がローカルブランチのみを検索しているため、リモートにのみ存在するブランチを検出できない
- UIは「Work on this」を表示し、ユーザーがクリックすると `create_linked_branch()` → `gh issue develop` が失敗する
- 正しくは「Switch to Worktree」を表示し、既存のワークツリー作成フローを使うべき

## ユーザーシナリオとテスト

### ユーザーストーリー 1 - リモートブランチ検出 (優先度: P0)

ユーザーとして、リモートにすでにIssue用ブランチが存在する場合に「Switch to Worktree」が表示され、ブランチ作成エラーを回避したい。

**独立したテスト**: `find_branch_for_issue` がリモートブランチを検出すること

**受け入れシナリオ**:

1. **前提条件** リモートに `feature/issue-42` が存在しローカルには存在しない、**操作** `find_branch_for_issue(repo, 42)` を呼ぶ、**期待結果** `Some("feature/issue-42")` が返る
2. **前提条件** ローカルに `feature/issue-42` が存在する、**操作** `find_branch_for_issue(repo, 42)` を呼ぶ、**期待結果** ローカルブランチが優先して返る（既存動作維持）

### ユーザーストーリー 2 - ブランチ既存時のエラーハンドリング (優先度: P1)

ユーザーとして、`gh issue develop` がブランチ既存エラーを返した場合でもエラーにならず成功扱いにしてほしい。

**独立したテスト**: `create_linked_branch` が "already exists" を含むstderrを成功として扱うこと

**受け入れシナリオ**:

1. **前提条件** リモートにブランチが既に存在する、**操作** `create_linked_branch()` を呼ぶ、**期待結果** `Ok(())` が返る

## エッジケース

- リモートブランチ名に `/` を含むプレフィックス（例: `origin/feature/issue-42`）の正しいストリップ
- 複数リモートに同名ブランチが存在する場合（最初にマッチしたものを使用）

## 要件

### 機能要件

- **FR-001**: `find_branch_for_issue()` はローカルブランチ検索後、見つからない場合に `git branch -r --list` でリモートブランチも検索する
- **FR-002**: リモートブランチ名から `origin/` 等のリモートプレフィックスを `split_once('/')` で除去して返す
- **FR-003**: `create_linked_branch()` は `gh issue develop` の stderr に "already exists" が含まれる場合、成功として扱う

### 非機能要件

- **NFR-001**: 既存のローカルブランチ検索の動作に影響を与えない

## 制約と仮定

- リモートブランチのプレフィックスは `{remote_name}/` 形式（標準的なgit動作）
- `git branch -r` はネットワーク不要（ローカルのリモート追跡ブランチを参照）

## 成功基準

- **SC-001**: `cargo test -p gwt-core` が全テスト通過
- **SC-002**: `cargo clippy --all-targets --all-features -- -D warnings` がエラーなし
- **SC-003**: リモートにのみブランチが存在するIssueでUIが「Switch to Worktree」を表示する
