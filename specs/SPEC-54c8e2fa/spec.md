# 機能仕様: Issue連携ブランチのリンク保証と起動フロー一元化

**仕様ID**: `SPEC-54c8e2fa`
**作成日**: 2026-02-26
**更新日**: 2026-02-26
**ステータス**: ドラフト
**カテゴリ**: Core / GUI / Launch Flow
**依存仕様**:

- SPEC-c6ba640a（Issue連携によるブランチ作成）
- SPEC-a4fb2db2（リモート起点Worktree作成）
- SPEC-rb01a2f3（Issueブランチ検出）

**入力**: ユーザー説明: "Issueからブランチを作成すると linked branch にならず通常ブランチになる。以前は機能していた。"

## 背景

- 現在は `start_launch_job` が先に通常のブランチ/Worktree作成を行い、成功後にフロントエンドから `link_branch_to_issue` を後追い実行している
- `create_linked_branch()` は `gh issue develop` が "already exists" を返した場合に成功扱いにしており、実際には未リンクブランチでも見逃してしまう
- その結果、Issue起点起動が成功しても GitHub Issue の Development セクションに表示されないブランチが生成される

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - Issue起点ブランチは常にリンク保証 (優先度: P0)

ユーザーとして、Issueから起動したブランチは必ずGitHub上でIssue linked branchとして作成/検証されてほしい。

**独立したテスト**: Issue起点 launch でバックエンドが link 処理を先に実行し、リンク不可時は起動失敗になること

**受け入れシナリオ**:

1. **前提条件** Issue起点で新規ブランチ名を指定、**操作** Launch 実行、**期待結果** `gh issue develop` 相当のリンク処理が先に行われてから Worktree 作成が実行される
2. **前提条件** ブランチが既存だがIssueに未リンク、**操作** Launch 実行、**期待結果** 起動は失敗し未リンク通常ブランチ成功にはならない

---

### ユーザーストーリー 2 - 既存リンク済みブランチの再利用 (優先度: P0)

ユーザーとして、同名ブランチが既に Issue にリンク済みなら再作成エラーではなく再利用したい。

**独立したテスト**: `gh issue develop` が "already exists" を返しても、`gh issue develop --list` でリンク確認できた場合は成功扱いになること

**受け入れシナリオ**:

1. **前提条件** `feature/issue-42` が既に Issue #42 にリンク済み、**操作** Launch 実行、**期待結果** ブランチ再利用で起動成功する
2. **前提条件** `feature/issue-42` が存在するが Issue #42 に未リンク、**操作** Launch 実行、**期待結果** `[E1012]` 相当で失敗する

---

### ユーザーストーリー 3 - 後段失敗時の安全ロールバック (優先度: P1)

ユーザーとして、Issueリンク後に起動が失敗/キャンセルした場合でも、今回新規に作成したブランチだけ安全に巻き戻してほしい。

**独立したテスト**: 新規作成時のみ rollback、再利用時は削除しないこと

**受け入れシナリオ**:

1. **前提条件** 今回新規作成したIssueブランチで launch 後段が失敗、**操作** rollback 実行、**期待結果** ローカル/リモートの当該新規ブランチのみ削除される
2. **前提条件** 既存リンク済みブランチを再利用した launch が失敗、**操作** rollback 実行、**期待結果** 既存ブランチは削除されない

## エッジケース

- `gh issue develop --list` の出力形式差異（owner/repo:branch, refs/heads/* など）でも対象ブランチ判定できること
- base 指定が `origin/develop` のような remote付き指定でも link 作成に利用できること
- Issue起点以外の通常 Launch（issueNumberなし）は既存フローを維持すること

## 要件 *(必須)*

### 機能要件

- **FR-001**: `LaunchAgentRequest` は `issueNumber` を backend に渡せなければならない
- **FR-002**: backend launch フローは `createBranch + issueNumber` の場合、通常作成ではなく Issueリンク先行フローを使用しなければならない
- **FR-003**: `gh issue develop` が "already exists" のとき、`gh issue develop --list` で対象ブランチが本当にIssueリンク済みか検証しなければならない
- **FR-004**: 既存ブランチが未リンクと判定された場合、起動は失敗し `E1012` エラーを返さなければならない
- **FR-005**: Issue起点起動の rollback は「今回新規作成したブランチ」のみに適用し、既存再利用ブランチを削除してはならない
- **FR-006**: フロントエンド後追いの `link_branch_to_issue` / `rollback_issue_branch` 依存を撤去し、launch 完了イベントではリンク処理を再実行しない

### 非機能要件

- **NFR-001**: Issue起点以外の Launch 挙動・エラー表示を変えない
- **NFR-002**: 既存の `gh issue develop` 連携コマンド（`link_branch_to_issue`）との互換性を維持する

## 制約と仮定

- `gh issue develop --list` が利用可能である（`gh` が利用可能/認証済み前提）
- ブランチのリンク判定は GitHub の authoritative な情報（`gh issue develop --list` 出力）を基準にする

## 成功基準 *(必須)*

- **SC-001**: Issue起点 launch で未リンク既存ブランチが成功扱いにならない
- **SC-002**: 既存リンク済みブランチは再利用して launch 成功できる
- **SC-003**: `cargo test -p gwt-core` と `cargo test -p gwt-tauri` の追加テストが通る
- **SC-004**: `gwt-gui` の該当テスト（Issue launch followup系）が通る
