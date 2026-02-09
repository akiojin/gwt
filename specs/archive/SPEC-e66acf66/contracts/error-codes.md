# エラーコード一覧

**仕様ID**: `SPEC-e66acf66` | **日付**: 2026-01-22

## コード体系

形式: `GWT-EXXX`
- `GWT`: アプリケーション識別子
- `E`: エラー（Error）
- `XXX`: 3桁の番号

## カテゴリ別コード一覧

### Worktree関連 (E001-E010)

| コード | 名前 | 説明 | サジェスチョン |
|--------|------|------|---------------|
| GWT-E001 | WorktreeCreateFailed | Worktree作成に失敗 | Check if branch exists, Try: git fetch --all |
| GWT-E002 | WorktreeDeleteFailed | Worktree削除に失敗 | Check if worktree is in use, Try: git worktree prune |
| GWT-E003 | WorktreeNotFound | Worktreeが見つからない | List worktrees: git worktree list |

### Git関連 (E011-E020)

| コード | 名前 | 説明 | サジェスチョン |
|--------|------|------|---------------|
| GWT-E011 | GitCommandFailed | Gitコマンド実行に失敗 | Check git installation, Verify repository |
| GWT-E012 | BranchNotFound | ブランチが存在しない | Fetch branches: git fetch --all, Check spelling |
| GWT-E013 | MergeConflict | マージコンフリクトが発生 | Resolve conflicts manually, Try: git status |

### 設定関連 (E021-E030)

| コード | 名前 | 説明 | サジェスチョン |
|--------|------|------|---------------|
| GWT-E021 | ConfigSaveFailed | 設定の保存に失敗 | Check file permissions, Verify disk space |
| GWT-E022 | ConfigLoadFailed | 設定の読み込みに失敗 | Check config file exists, Verify JSON syntax |
| GWT-E023 | ProfileNotFound | プロファイルが見つからない | Create profile first, Check profile name |

### AI関連 (E031-E040)

| コード | 名前 | 説明 | サジェスチョン |
|--------|------|------|---------------|
| GWT-E031 | AiApiCallFailed | AI APIの呼び出しに失敗 | Check network connection, Verify API endpoint |
| GWT-E032 | AiApiKeyMissing | APIキーが設定されていない | Set API key in settings, Check environment variables |
| GWT-E033 | AiResponseInvalid | AIレスポンスが不正 | Retry the request, Check API status |

### システム関連 (E041-E050)

| コード | 名前 | 説明 | サジェスチョン |
|--------|------|------|---------------|
| GWT-E041 | FileOperationFailed | ファイル操作に失敗 | Check file path, Verify permissions |
| GWT-E042 | PermissionDenied | 権限エラー | Check file permissions, Try: chmod +x |
| GWT-E043 | NetworkError | ネットワークエラー | Check internet connection, Verify proxy settings |

### その他 (E999)

| コード | 名前 | 説明 | サジェスチョン |
|--------|------|------|---------------|
| GWT-E999 | Unknown | 不明なエラー | Check logs for details, Report issue |

## 使用例

### エラー発生時

```rust
use gwt_core::error::{ErrorCode, ErrorState};

// ErrorCodeからErrorStateを生成
let error = ErrorState::from_code(
    ErrorCode::WorktreeCreateFailed,
    "Failed to create worktree for branch 'feature/foo'".to_string(),
    Some("Branch does not exist in remote".to_string()),
);

// エラーキューに追加
app.error_queue.push(error);
```

### ログ出力時

```rust
use tracing::error;
use gwt_core::error::ErrorCode;

let code = ErrorCode::WorktreeCreateFailed;
error!(
    code = %code.as_str(),
    category = %code.category().as_str(),
    message = "Failed to create worktree",
    details = "Branch does not exist",
    "Error occurred"
);
```
