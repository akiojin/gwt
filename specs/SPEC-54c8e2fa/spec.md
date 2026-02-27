# 機能仕様: Issue連携ブランチのリンク保証と起動フロー一元化

**仕様ID**: `SPEC-54c8e2fa`
**作成日**: 2026-02-26
**更新日**: 2026-02-27
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
- Issue #1278 では `branch..gh-merge-base` の不正な Git 設定が残存し、`git for-each-ref` が失敗してブランチ一覧取得全体が停止する不具合が発生した

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

---

### ユーザーストーリー 4 - 壊れた gh-merge-base 設定からの自動復旧 (優先度: P0)

ユーザーとして、Issue連携起点で壊れた Git config が残っていても、ブランチ一覧表示が停止せず自動復旧してほしい。

**独立したテスト**: `.git/config` に `branch..gh-merge-base` または `[branch ""]` を注入した状態でも `Branch::list` / `Branch::list_remote` が成功すること

**受け入れシナリオ**:

1. **前提条件** repo config に `branch..gh-merge-base` が存在、**操作** ブランチ一覧を取得、**期待結果** 不正設定を除去して再試行し、一覧取得が成功する
2. **前提条件** repo config に `[branch ""]` セクションが存在、**操作** ブランチ一覧を取得、**期待結果** 空ブランチセクションを除去して再試行し、一覧取得が成功する

---

### ユーザーストーリー 5 - 空ブランチ名入力の事前拒否 (優先度: P1)

ユーザーとして、Issue連携ブランチ作成時に空ブランチ名が渡されたら `gh issue develop` 実行前に明確に失敗してほしい。

**独立したテスト**: `create_or_verify_linked_branch` が空白のみの `branch_name` を即時エラーとし、`gh` 実行に進まないこと

**受け入れシナリオ**:

1. **前提条件** `branch_name=""`（または空白のみ）、**操作** Issue連携ブランチ作成 API を呼ぶ、**期待結果** 入力検証エラーを返し外部コマンド実行を行わない

## エッジケース

- `gh issue develop --list` の出力形式差異（owner/repo:branch, refs/heads/* など）でも対象ブランチ判定できること
- base 指定が `origin/develop` のような remote付き指定でも link 作成に利用できること
- Issue起点以外の通常 Launch（issueNumberなし）は既存フローを維持すること
- Worktree (`.git` がファイル) でも実体 `gitdir/config` を解決して修復対象を特定できること
- 自動修復は `branch..gh-merge-base` 系のみを対象とし、global/system 設定や他キーに作用しないこと

## 要件 *(必須)*

### 機能要件

- **FR-001**: `LaunchAgentRequest` は `issueNumber` を backend に渡せなければならない
- **FR-002**: backend launch フローは `createBranch + issueNumber` の場合、通常作成ではなく Issueリンク先行フローを使用しなければならない
- **FR-003**: `gh issue develop` が "already exists" のとき、`gh issue develop --list` で対象ブランチが本当にIssueリンク済みか検証しなければならない
- **FR-004**: 既存ブランチが未リンクと判定された場合、起動は失敗し `E1012` エラーを返さなければならない
- **FR-005**: Issue起点起動の rollback は「今回新規作成したブランチ」のみに適用し、既存再利用ブランチを削除してはならない
- **FR-006**: フロントエンド後追いの `link_branch_to_issue` / `rollback_issue_branch` 依存を撤去し、launch 完了イベントではリンク処理を再実行しない
- **FR-007**: `Branch::list` / `Branch::list_remote` の `for-each-ref` が `bad config variable` かつ `gh-merge-base` を含む失敗時、ローカル repo config の修復を試行しなければならない
- **FR-008**: 修復処理は `branch..gh-merge-base` の直接キーと `[branch ""]` セクションのみを除去しなければならない
- **FR-009**: 修復後の `for-each-ref` 再試行は1回のみ行い、再失敗時は通常の `E1013` エラーを返さなければならない
- **FR-010**: `create_or_verify_linked_branch` は `branch_name` が空白のみの場合、`gh issue develop` 実行前に入力検証エラーを返さなければならない
- **FR-011 (TDD)**: FR-007〜FR-010 は再現テストを先に追加して RED を確認した後に実装して GREEN 化しなければならない

### 非機能要件

- **NFR-001**: Issue起点以外の Launch 挙動・エラー表示を変えない
- **NFR-002**: 既存の `gh issue develop` 連携コマンド（`link_branch_to_issue`）との互換性を維持する
- **NFR-003**: config 自動修復は対象 repo のローカル config のみに限定し、global/system config を更新してはならない

## 制約と仮定

- `gh issue develop --list` が利用可能である（`gh` が利用可能/認証済み前提）
- ブランチのリンク判定は GitHub の authoritative な情報（`gh issue develop --list` 出力）を基準にする
- `bad config variable 'branch..gh-merge-base'` は `gh issue develop` によって残存しうる既知不正状態として扱う

## 成功基準 *(必須)*

- **SC-001**: Issue起点 launch で未リンク既存ブランチが成功扱いにならない
- **SC-002**: 既存リンク済みブランチは再利用して launch 成功できる
- **SC-003**: `cargo test -p gwt-core` と `cargo test -p gwt-tauri` の追加テストが通る
- **SC-004**: `gwt-gui` の該当テスト（Issue launch followup系）が通る
- **SC-005**: `branch..gh-merge-base` を含む repo config を持つテストで `Branch::list` と `Branch::list_remote` が成功する
- **SC-006**: 空ブランチ名入力時に `create_or_verify_linked_branch` が即時エラーを返し、`gh` 未実行で終了する
