<!-- markdownlint-disable MD013 -->
# データモデル設計: AIBranchSuggest

**仕様ID**: `SPEC-1ad9c07d` | **日付**: 2026-02-08

## 新規エンティティ

### AIBranchSuggestPhase

AIBranchSuggestステップ内のサブフェーズを表すenum。

| フィールド | 型 | 説明 |
|-----------|-----|------|
| Input | - | テキスト入力フェーズ。ユーザーがブランチ目的を入力 |
| Loading | - | AI APIリクエスト中のローディングフェーズ |
| Select | - | 3候補からの選択フェーズ |
| Error | - | APIエラー表示フェーズ |

### AIBranchSuggestState（WizardStateに埋め込み）

WizardStateに追加するフィールド群。

| フィールド | 型 | デフォルト | 説明 |
|-----------|-----|-----------|------|
| ai_enabled | bool | false | AI設定が有効かどうか（wizard open時に設定） |
| ai_branch_phase | AIBranchSuggestPhase | Input | 現在のサブフェーズ |
| ai_branch_input | String | "" | ブランチ目的の入力テキスト |
| ai_branch_cursor | usize | 0 | 入力テキストのカーソル位置 |
| ai_branch_suggestions | Vec\<String\> | [] | AI生成のブランチ名候補（プレフィックス込み、最大3件） |
| ai_branch_selected | usize | 0 | 選択中の候補インデックス |
| ai_branch_error | Option\<String\> | None | エラーメッセージ |

## 既存エンティティの変更

### WizardStep enum

新しいバリアントを追加:

```text
変更前: ... IssueSelect, BranchNameInput, ...
変更後: ... IssueSelect, AIBranchSuggest, BranchNameInput, ...
```

### BranchType enum

新しいメソッドを追加:

```text
from_prefix(name: &str) -> Option<(BranchType, &str)>
  入力: プレフィックス込みのブランチ名（例: "feature/add-login"）
  出力: (BranchType, プレフィックスを除いた名前)
  マッチしない場合: None を返し、呼び出し元で現在のBranchTypeを維持
```

### WizardState struct

上記「AIBranchSuggestState」のフィールド群を追加。

## 非同期通信チャネル

### AIBranchSuggestの通信

app.rs側に追加:

| フィールド | 型 | 説明 |
|-----------|-----|------|
| ai_branch_suggest_rx | Option\<mpsc::Receiver\<AiBranchSuggestUpdate\>\> | AI候補受信チャネル |

### AiBranchSuggestUpdate

| フィールド | 型 | 説明 |
|-----------|-----|------|
| result | Result\<Vec\<String\>, AIError\> | 候補リストまたはエラー |

## データフロー

```text
1. ユーザーが目的テキストを入力 → ai_branch_input に格納
2. Enter押下 → app.rs が AI設定を取得し thread::spawn で API 呼び出し
3. AI レスポンス（JSON）をパース → Vec<String> に変換
4. 各候補のsuffix（プレフィックスを除いた部分）に sanitize_branch_name() を適用（プレフィックスは保持）
5. ai_branch_suggestions に格納、phase を Select に遷移
6. ユーザーが候補を選択 → from_prefix() でタイプと名前を分離
7. branch_type と new_branch_name を更新
8. BranchNameInput ステップに遷移
```

## AI API リクエスト/レスポンス仕様

### システムメッセージ

```text
You are a git branch naming assistant. Generate exactly 3 branch name suggestions
based on the user's description. Each suggestion must include the branch type prefix.
Use lowercase, hyphens for separators, and keep names concise (under 50 characters
including prefix). Respond in JSON format only.
```

### ユーザーメッセージ

```text
Branch type: {branch_type_prefix}
Description: {user_input}

Respond with JSON: {"suggestions": ["prefix/name-1", "prefix/name-2", "prefix/name-3"]}
```

### レスポンスパース

1. レスポンステキストからJSON部分を抽出（最初の`{`から最後の`}`まで）
2. `{"suggestions": [...]}` をパース
3. suggestionsはちょうど3件でなければならない
4. 各候補は許可プレフィックス（feature/bugfix/hotfix/release）のいずれかを含む必要がある
5. suffixにのみ `sanitize_branch_name()` を適用し、空/無効な候補はエラーとして扱う

## 関連図

```text
WizardState
├── step: WizardStep (← AIBranchSuggest追加)
├── branch_type: BranchType (← from_prefix()追加)
├── new_branch_name: String
├── ai_enabled: bool (← 新規)
├── ai_branch_phase: AIBranchSuggestPhase (← 新規)
├── ai_branch_input: String (← 新規)
├── ai_branch_cursor: usize (← 新規)
├── ai_branch_suggestions: Vec<String> (← 新規)
├── ai_branch_selected: usize (← 新規)
└── ai_branch_error: Option<String> (← 新規)

app.rs Model
└── ai_branch_suggest_rx: Option<Receiver<AiBranchSuggestUpdate>> (← 新規)
```
