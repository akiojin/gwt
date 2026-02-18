<!-- markdownlint-disable MD013 -->
# AIBranchSuggest API契約

**仕様ID**: `SPEC-1ad9c07d` | **日付**: 2026-02-08

## 内部インターフェース

### AIBranchSuggestPhase enum

```text
enum AIBranchSuggestPhase {
    Input,    // テキスト入力待ち
    Loading,  // API呼び出し中
    Select,   // 候補選択
    Error,    // エラー表示
}
```

Default: `Input`

### WizardState 追加フィールド

```text
ai_enabled: bool                          // AI設定有効フラグ
ai_branch_phase: AIBranchSuggestPhase     // サブフェーズ
ai_branch_input: String                   // 目的入力テキスト
ai_branch_cursor: usize                   // カーソル位置
ai_branch_suggestions: Vec<String>        // 候補リスト（プレフィックス込み）
ai_branch_selected: usize                 // 選択インデックス
ai_branch_error: Option<String>           // エラーメッセージ
```

### BranchType::from_prefix()

```text
入力: name: &str (例: "feature/add-login-page")
出力: Option<(BranchType, &str)>
  Some((BranchType::Feature, "add-login-page"))  // プレフィックスがマッチ
  None                                            // マッチしない
```

### AiBranchSuggestUpdate

```text
struct AiBranchSuggestUpdate {
    result: Result<Vec<String>, AIError>
}
```

## AI API契約

### リクエスト

既存の `AIClient::create_response()` を使用。

メッセージ構成:

| 順序 | role | content |
|------|------|---------|
| 1 | system | ブランチ名生成の指示（JSON出力形式指定） |
| 2 | user | ブランチタイプ + ユーザー入力の目的説明 |

### レスポンス

JSON形式:

```json
{
  "suggestions": [
    "feature/add-oauth-login",
    "feature/implement-login-page",
    "feature/oauth-authentication"
  ]
}
```

### エラーケース

| エラー種別 | AIError variant | フォールバック動作 |
|-----------|----------------|-------------------|
| 認証エラー | Unauthorized | Error フェーズ表示 → Enter で手動入力 |
| レート制限 | RateLimited | Error フェーズ表示 → Enter で手動入力 |
| サーバーエラー | ServerError | Error フェーズ表示 → Enter で手動入力 |
| ネットワークエラー | NetworkError | Error フェーズ表示 → Enter で手動入力 |
| パースエラー | ParseError | Error フェーズ表示 → Enter で手動入力 |
| 設定エラー | ConfigError | Error フェーズ表示 → Enter で手動入力 |

### レスポンスパース（期待仕様）

- レスポンステキストから **最初の `{` から最後の `}`** までを抽出し、JSONとしてパースする
- 形式は `{"suggestions": [...]}` のみを許可する
- `suggestions` は **ちょうど3件** でなければならない
- 各候補は `feature/|bugfix/|hotfix/|release/` のいずれかのプレフィックスを含む必要がある（含まない候補は無効）
- サニタイズは **プレフィックスを除いたsuffixにのみ** `sanitize_branch_name()` を適用し、プレフィックスは保持する

## イベントフロー

### 入力フェーズ (Input)

| イベント | 動作 |
|---------|------|
| 文字入力 | ai_branch_input に追加、カーソル移動 |
| Backspace | ai_branch_input から削除、カーソル移動 |
| Enter | 入力が空でなければ Loading フェーズへ遷移、API呼び出し開始 |
| Enter (空入力) | 何もしない（入力を促す） |
| Esc | BranchNameInput ステップにスキップ（new_branch_nameは現在値を保持。未設定なら空のまま） |
| Left/Right | カーソル移動 |

### ローディングフェーズ (Loading)

| イベント | 動作 |
|---------|------|
| Esc | リクエスト結果を無視して Input フェーズに戻る（HTTPリクエスト自体は継続する場合がある） |
| API成功 | suggestions を設定、Select フェーズに遷移 |
| APIエラー | error を設定、Error フェーズに遷移 |

### 選択フェーズ (Select)

| イベント | 動作 |
|---------|------|
| Up/Down | 候補選択を移動 |
| Enter | 候補を確定、branch_type/new_branch_name を更新、BranchNameInput に遷移 |
| Esc | Input フェーズに戻る |
| マウスクリック | 候補を選択 |
| マウスダブルクリック | 候補を確定 |

### エラーフェーズ (Error)

| イベント | 動作 |
|---------|------|
| Enter | BranchNameInput ステップにスキップ（new_branch_nameは現在値を保持） |
| Esc | Input フェーズに戻る |

## レンダリング仕様

### 入力フェーズ

```text
What is this branch for?

[入力テキスト|カーソル]

[Enter] Generate  [Esc] Skip
```

### ローディングフェーズ

```text
Generating branch name suggestions...

[Esc] Cancel
```

### 選択フェーズ

```text
Select a branch name:

> feature/add-oauth-login
  feature/implement-login-page
  feature/oauth-authentication

[Enter] Select  [Esc] Back  [Up/Down] Navigate
```

### エラーフェーズ

```text
Error: [エラーメッセージ]

[Enter] Manual input  [Esc] Retry
```
