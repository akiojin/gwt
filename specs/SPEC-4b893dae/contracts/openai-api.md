# API契約: OpenAI互換API

**仕様ID**: `SPEC-4b893dae` | **日付**: 2026-01-19

## 概要

AIサマリー機能で使用するOpenAI互換APIの仕様を定義する。OpenAI API、Azure OpenAI、Ollama等のOpenAI互換エンドポイントに対応。

## エンドポイント

### Chat Completions

```
POST {endpoint}/chat/completions
```

## リクエスト

### ヘッダー

| ヘッダー      | 値                | 必須  |
| ------------- | ----------------- | ----- |
| Content-Type  | application/json  | Yes   |
| Authorization | Bearer {api_key}  | Yes*  |

*注: api_keyが空の場合はAuthorizationヘッダーを省略（ローカルLLM用）

### ボディ

```json
{
  "model": "gpt-4o-mini",
  "messages": [
    {
      "role": "system",
      "content": "You are a helpful assistant that summarizes git commit history. Respond with 2-3 bullet points in English, each starting with '- '. Be concise and focus on the main changes."
    },
    {
      "role": "user",
      "content": "Summarize the following git commits for branch 'feature/add-login':\n\na1b2c3d Add login form validation\nb2c3d4e Implement JWT authentication\nc3d4e5f Fix password reset flow"
    }
  ],
  "max_tokens": 150,
  "temperature": 0.3
}
```

### パラメータ詳細

| パラメータ  | 型      | 説明                     | デフォルト  |
| ----------- | ------- | ------------------------ | ----------- |
| model       | string  | モデル名                 | gpt-4o-mini |
| messages    | array   | メッセージ配列           | -           |
| max_tokens  | integer | 最大トークン数           | 150         |
| temperature | number  | 生成の多様性（0.0-2.0）  | 0.3         |

## レスポンス

### 成功時 (200 OK)

```json
{
  "id": "chatcmpl-abc123",
  "object": "chat.completion",
  "created": 1706745600,
  "model": "gpt-4o-mini",
  "choices": [
    {
      "index": 0,
      "message": {
        "role": "assistant",
        "content": "- Added user authentication with JWT tokens and login form validation\n- Implemented password reset functionality with email verification\n- Enhanced security measures for user session management"
      },
      "finish_reason": "stop"
    }
  ],
  "usage": {
    "prompt_tokens": 85,
    "completion_tokens": 42,
    "total_tokens": 127
  }
}
```

### エラー時

#### 401 Unauthorized

```json
{
  "error": {
    "message": "Invalid API key",
    "type": "invalid_request_error",
    "code": "invalid_api_key"
  }
}
```

#### 429 Rate Limited

```json
{
  "error": {
    "message": "Rate limit exceeded",
    "type": "rate_limit_error",
    "code": "rate_limit_exceeded"
  }
}
```

#### 500 Internal Server Error

```json
{
  "error": {
    "message": "Internal server error",
    "type": "server_error",
    "code": "internal_error"
  }
}
```

## プロンプトテンプレート

### システムプロンプト

```text
You are a helpful assistant that summarizes git commit history.
Respond with 2-3 bullet points in English, each starting with '- '.
Be concise and focus on the main changes.
Do not include commit hashes or dates in the summary.
```

### ユーザープロンプト

```text
Summarize the following git commits for branch '{branch_name}':

{commit_list}
```

変数:
- `{branch_name}`: ブランチ名（例: `feature/add-login`）
- `{commit_list}`: コミット一覧（1行1コミット、ハッシュ+メッセージ形式）

## クライアント実装要件

### タイムアウト

| 操作       | タイムアウト |
| ---------- | ------------ |
| 接続       | 5秒          |
| 読み取り   | 30秒         |

### リトライ

| エラー           | リトライ | 間隔                          |
| ---------------- | -------- | ----------------------------- |
| 429 Rate Limited | 最大3回  | 指数バックオフ（1s, 2s, 4s）  |
| 5xx Server Error | 最大2回  | 1秒固定                       |
| 接続エラー       | 最大2回  | 1秒固定                       |
| 401/403          | なし     | -                             |

### エラーハンドリング

```rust
pub enum AIError {
    /// APIキーが無効
    Unauthorized,
    /// レート制限
    RateLimited { retry_after: Option<u64> },
    /// サーバーエラー
    ServerError(String),
    /// ネットワークエラー
    NetworkError(String),
    /// レスポンスパースエラー
    ParseError(String),
    /// 設定エラー（エンドポイント未設定等）
    ConfigError(String),
}
```

## 互換性マトリクス

| プロバイダー | エンドポイント                                                         | 認証           | 備考             |
| ------------ | ---------------------------------------------------------------------- | -------------- | ---------------- |
| OpenAI       | `https://api.openai.com/v1`                                            | Bearer token   | 標準             |
| Azure OpenAI | `https://{resource}.openai.azure.com/openai/deployments/{deployment}`  | api-key header | パス形式が異なる |
| Ollama       | `http://localhost:11434/v1`                                            | なし           | ローカル         |
| LM Studio    | `http://localhost:1234/v1`                                             | なし           | ローカル         |
| vLLM         | `http://localhost:8000/v1`                                             | なし           | ローカル         |

## セキュリティ考慮事項

1. **APIキー管理**
   - プロファイルYAMLに保存（ファイルパーミッション0o600）
   - 環境変数からのフォールバック
   - ログにAPIキーを出力しない

2. **データプライバシー**
   - コミットメッセージのみをAPIに送信
   - コード内容は送信しない
   - ユーザーの同意なくデータを送信しない（AI設定が有効な場合のみ）

3. **ネットワーク**
   - HTTPS必須（ローカルLLM除く）
   - 証明書検証を無効化しない
