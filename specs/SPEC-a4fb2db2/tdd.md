# TDD テスト仕様: GitHub リモート起点の Worktree 作成

**仕様ID**: `SPEC-a4fb2db2`

## テスト対象

`crates/gwt-core/src/git/gh_cli.rs` の `#[cfg(test)]` モジュール

## テスト一覧

### シグネチャテスト（型コンパイルテスト）

#### `create_remote_branch_returns_result`

```text
目的: create_remote_branch の関数シグネチャが期待通りの型を返すことを確認
パターン: delete_remote_branch_returns_result と同一
検証: fn(&Path, &str, &str) -> Result<(), String> にキャスト可能
```

#### `resolve_remote_branch_sha_returns_result`

```text
目的: resolve_remote_branch_sha の関数シグネチャが期待通りの型を返すことを確認
検証: fn(&Path, &str) -> Result<String, String> にキャスト可能
```

### エンドポイントフォーマットテスト

#### `create_remote_branch_endpoint_format`

```text
目的: create_remote_branch が使用する API エンドポイントの文字列が正しいことを確認
入力: owner="akiojin", repo="gwt", branch="feature/test"
期待: "repos/akiojin/gwt/git/refs"
```

#### `resolve_remote_branch_sha_endpoint_format`

```text
目的: resolve_remote_branch_sha が使用する API エンドポイントの文字列が正しいことを確認
入力: owner="akiojin", repo="gwt", branch="develop"
期待: "repos/akiojin/gwt/git/ref/heads/develop"
```

### JSON パーステスト

#### `resolve_remote_branch_sha_parses_json`

```text
目的: GitHub API レスポンス JSON から SHA を正しく抽出できることを確認
入力: {"ref": "refs/heads/develop", "object": {"sha": "abc123def456", "type": "commit"}}
期待: "abc123def456"
```

#### `resolve_remote_branch_sha_invalid_json`

```text
目的: 不正な JSON でエラーを返すことを確認
入力: "not valid json"
期待: Err
```

#### `resolve_remote_branch_sha_missing_sha`

```text
目的: SHA フィールドが存在しない JSON でエラーを返すことを確認
入力: {"ref": "refs/heads/develop", "object": {"type": "commit"}}
期待: Err
```
