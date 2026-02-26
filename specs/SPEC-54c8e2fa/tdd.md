# TDD テスト仕様: Issue連携ブランチのリンク保証と起動フロー一元化

**仕様ID**: `SPEC-54c8e2fa`

## テスト対象

- `crates/gwt-core/src/git/issue.rs`
- `crates/gwt-tauri/src/commands/terminal.rs`
- `gwt-gui/src/App.svelte` 関連ユニットテスト

## REDで追加するテスト

### gwt-core (`issue.rs`)

#### `test_issue_develop_args_with_base`

```text
目的: base 指定時に --base が付与されることを確認
入力: issue=42, branch="feature/issue-42", base=Some("develop")
期待: args に ["--base", "develop"] が含まれる
```

#### `test_issue_develop_args_without_base`

```text
目的: base 未指定時は --base が付与されないことを確認
入力: base=None
期待: args に "--base" が存在しない
```

#### `test_issue_develop_list_mentions_branch_owner_repo_colon`

```text
目的: "owner/repo:feature/issue-42" 形式を linked 判定できることを確認
```

#### `test_issue_develop_list_mentions_branch_refs_heads`

```text
目的: "refs/heads/feature/issue-42" 形式を linked 判定できることを確認
```

#### `test_issue_develop_list_does_not_match_other_issue_branch`

```text
目的: 近い名前（feature/issue-420 等）を誤判定しないことを確認
```

### gwt-tauri (`terminal.rs`)

#### `make_request_sets_issue_number_none_by_default`

```text
目的: LaunchAgentRequest の既存生成ヘルパで issue_number が None であることを確認
```

#### `launch_request_deserialize_with_issue_number`

```text
目的: issueNumber(JSON) が issue_number にデシリアライズされることを確認
```

### gwt-gui

#### `issue-launch-followup-is-not-queued-in-app`

```text
目的: launch-finished 後追い link/rollback が実行されない（設計撤去）ことを検証
期待: App.svelte から link_branch_to_issue invoke が行われない
```

## GREEN判定

- 上記 RED テストが実装後に全て通る
- 既存テストの主要回帰（Issue一覧→Work on this→Launch）に失敗がない
