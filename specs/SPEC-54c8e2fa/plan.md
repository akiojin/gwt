# 実装計画: Issue連携ブランチのリンク保証と起動フロー一元化

**仕様ID**: `SPEC-54c8e2fa`

## 概要

Issue起点 launch のブランチ作成/リンクを backend で一体化し、`linked branch` を保証する。`already exists` を単純成功扱いせず、実リンク確認を行う。

## 実装フェーズ

### Phase 1: gwt-core Issue API拡張

**ファイル**: `crates/gwt-core/src/git/issue.rs`, `crates/gwt-core/src/git.rs`

1. `IssueLinkedBranchStatus`（`Created` / `AlreadyLinked`）を追加
2. `create_or_verify_linked_branch(repo, issue, branch, base)` を追加
   - `gh issue develop ... --name ... --checkout=false [--base ...]` 実行
   - 成功時: `Created`
   - "already exists" 時: `gh issue develop --list` を実行してリンク検証
   - 検証成功: `AlreadyLinked`
   - 検証失敗: `[E1012] Issue branch exists but is not linked` エラー
3. 既存 `create_linked_branch()` は互換ラッパに変更（内部で新関数呼び出し）
4. `issue_develop_args` は `base` オプション対応

### Phase 2: gwt-tauri backend launch 一元化

**ファイル**: `crates/gwt-tauri/src/commands/terminal.rs`

1. `LaunchAgentRequest` に `issue_number: Option<u64>` を追加
2. launch create分岐で `create_branch + issue_number` の専用フローを追加
   - 先に `create_or_verify_linked_branch` を実行
   - `Created`: 新規作成フラグを立てて `resolve_worktree_path` へ
   - `AlreadyLinked`: 既存再利用として `resolve_worktree_path` へ
3. 失敗時 rollback で「今回新規作成」のみ remote/local 削除するロジックを追加
4. 通常（issueNumberなし）は既存 `create_new_worktree_path` フローを維持

### Phase 3: gwt-gui フォローアップ除去

**ファイル**: `gwt-gui/src/App.svelte`

1. `issueLaunchFollowups` キューと `queueIssueLaunchFollowup` を削除
2. `launch-finished` 時の `link_branch_to_issue` / `rollback_issue_branch` 呼び出しを削除
3. LaunchProgress表示は既存の job status 連携に限定

### Phase 4: テスト整備

1. `gwt-core` unit: args生成/判定ヘルパ/未リンクエラー分岐
2. `gwt-tauri` unit: request struct と issueNumber分岐の振る舞い検証
3. `gwt-gui` unit: launch後追い invoke が消えていることの回帰確認

## リスク

- `gh issue develop --list` の出力仕様差分で判定漏れが起こる可能性があるため、複数フォーマットに対応した判定関数を実装する
- backend rollback の条件を誤ると既存ブランチを削除する危険があるため、「新規作成フラグ」の明示管理を必須にする
