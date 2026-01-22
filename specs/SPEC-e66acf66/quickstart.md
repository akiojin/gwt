# クイックスタート: エラーポップアップ・ログ出力システム

**仕様ID**: `SPEC-e66acf66` | **日付**: 2026-01-22

## 開発者向けガイド

### エラーの発生させ方（テスト用）

```bash
# 存在しないブランチでWorktree作成を試みる
gwt
# TUIで存在しないブランチを選択してEnter

# または、不正な操作を実行
# - 存在しないWorktreeを削除しようとする
# - 権限のないディレクトリにWorktreeを作成しようとする
```

### ログファイルの確認

```bash
# ログディレクトリ
ls ~/.gwt/logs/

# 今日のログを確認
cat ~/.gwt/logs/gwt.jsonl.$(date +%Y-%m-%d)

# エラーのみフィルタ
cat ~/.gwt/logs/gwt.jsonl.$(date +%Y-%m-%d) | jq 'select(.level == "ERROR")'

# 特定のエラーコードで検索
cat ~/.gwt/logs/gwt.jsonl.$(date +%Y-%m-%d) | jq 'select(.code == "GWT-E001")'
```

### TUI内でのログ確認

```text
1. エラーポップアップが表示されている状態で [l] キーを押す
2. ログビューアー画面に遷移
3. 該当エラーが選択された状態で表示される
4. [Esc] で元の画面に戻る
```

## 新規エラーコード追加手順

### 1. ErrorCode enumに追加

```rust
// crates/gwt-core/src/error/codes.rs

pub enum ErrorCode {
    // ... 既存のコード

    // 新規追加
    NewErrorCode,  // GWT-E0XX
}
```

### 2. コード文字列のマッピング

```rust
impl ErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            // ... 既存のマッピング
            Self::NewErrorCode => "GWT-E0XX",
        }
    }
}
```

### 3. カテゴリのマッピング

```rust
impl ErrorCode {
    pub fn category(&self) -> ErrorCategory {
        match self {
            // ... 既存のマッピング
            Self::NewErrorCode => ErrorCategory::System, // 適切なカテゴリ
        }
    }
}
```

### 4. サジェスチョンの追加

```rust
// crates/gwt-core/src/error/suggestions.rs

pub fn get_suggestions(code: ErrorCode) -> Vec<&'static str> {
    match code {
        // ... 既存のサジェスチョン
        ErrorCode::NewErrorCode => vec![
            "First suggestion",
            "Try: some_command",
        ],
    }
}
```

### 5. ドキュメント更新

`specs/SPEC-e66acf66/contracts/error-codes.md` にエントリを追加。

## エラーハンドリングのベストプラクティス

### エラー発生時のコード例

```rust
use gwt_core::error::{ErrorCode, ErrorState};
use tracing::error;

fn create_worktree(branch: &str) -> Result<(), AppError> {
    match do_create_worktree(branch) {
        Ok(_) => Ok(()),
        Err(e) => {
            // 1. ログに出力
            let code = ErrorCode::WorktreeCreateFailed;
            error!(
                code = %code.as_str(),
                category = %code.category().as_str(),
                message = %e.to_string(),
                branch = %branch,
                "Failed to create worktree"
            );

            // 2. ポップアップ用のErrorStateを生成
            let error_state = ErrorState::from_code(
                code,
                format!("Failed to create worktree for '{}'", branch),
                Some(e.to_string()),
            );

            // 3. エラーキューに追加
            // (app.error_queue.push(error_state) は呼び出し元で実行)

            Err(AppError::WorktreeError(e))
        }
    }
}
```

### 重大度の使い分け

| 重大度 | 用途 | 例 |
|--------|------|-----|
| Error | 操作が完全に失敗 | Worktree作成失敗 |
| Warning | 操作は完了したが注意が必要 | 古いキャッシュを使用 |
| Info | ユーザーへの情報通知 | セッション期限切れ（再取得可能） |

## トラブルシューティング

### ポップアップが表示されない

1. エラーが発生しているか確認（ログファイルをチェック）
2. ErrorQueueにエラーが追加されているか確認
3. render関数でerror_queueがチェックされているか確認

### ログファイルが作成されない

1. ログディレクトリの権限を確認: `ls -la ~/.gwt/`
2. ディスク空き容量を確認
3. init_logger()が呼ばれているか確認

### クリップボードコピーが動作しない

1. プラットフォームのクリップボード機能を確認
2. Linux: X11/Waylandセッションを確認
3. SSH接続時: クリップボード転送が有効か確認
