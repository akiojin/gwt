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

#### `resolve_remote_branch_sha_endpoint_format_with_slash`

```text
目的: スラッシュを含むブランチ名でエンドポイントが正しいことを確認
入力: owner="akiojin", repo="gwt", branch="feature/test"
期待: "repos/akiojin/gwt/git/ref/heads/feature/test"
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

#### `resolve_remote_branch_sha_missing_object`

```text
目的: object フィールドが存在しない JSON でエラーを返すことを確認
入力: {"ref": "refs/heads/develop"}
期待: Err
```

### エラー分類テスト（classify_create_branch_error）

#### `classify_error_422_reference_already_exists`

```text
目的: "Reference already exists" を含むレスポンスで 422 エラーを正しく分類
入力: "Reference already exists", branch="feature/x"
期待: "already exists on remote" を含む
```

#### `classify_error_422_status_code`

```text
目的: "422" を含むレスポンスで既存ブランチエラーを正しく分類
入力: {"message":"Reference already exists","status":"422"}, branch="feat/y"
期待: "already exists on remote" を含む
```

#### `classify_error_403_forbidden`

```text
目的: "403 Forbidden" を含むレスポンスで権限エラーを正しく分類
入力: "403 Forbidden", branch="feat/z"
期待: "Permission denied" を含む
```

#### `classify_error_404_not_found`

```text
目的: "404 Not Found" を含むレスポンスでリポジトリ未発見エラーを正しく分類
入力: "404 Not Found", branch="feat/w"
期待: "not found on remote" を含む
```

#### `classify_error_unknown`

```text
目的: 未知のエラーレスポンスで汎用エラーメッセージを返す
入力: "something unexpected", branch="feat/u"
期待: "Failed to create remote branch" を含む
```

#### `classify_error_422_does_not_fallback`

```text
目的: 422 エラーのメッセージ形式が terminal.rs のフォールバック判定と整合
入力: "422", branch="feature/no-fallback"
期待: "already exists on remote" を含む（terminal.rs がこの文字列でフォールバック抑制する）
```
