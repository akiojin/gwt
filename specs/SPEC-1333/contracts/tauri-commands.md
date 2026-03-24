# Tauriコマンド契約: AI自動ブランチ命名モード

**仕様ID**: `SPEC-9cd50c7c` | **日付**: 2026-02-26

## 変更: suggest_branch_name（旧: suggest_branch_names）

### リクエスト

```
command: "suggest_branch_name"
params:
  description: String  // ユーザーが入力した説明文（空不可）
```

### レスポンス

```
BranchSuggestResult:
  status: "ok"
    suggestion: "feature/add-user-login"  // 完全なブランチ名（prefix付き）
    error: null

  status: "ai-not-configured"
    suggestion: ""
    error: null

  status: "error"
    suggestion: ""
    error: "Failed to generate branch name: ..."
```

## 変更: start_launch_job

### リクエスト（追加フィールド）

```
LaunchAgentRequest:
  ...（既存フィールド）
  aiBranchDescription?: string  // AI Suggestモード時の説明文
```

### 振る舞い

- `aiBranchDescription` が存在する場合:
  - "create" ステップ内でAIブランチ名生成を実行
  - 生成成功: そのブランチ名でworktree作成
  - 生成失敗: `[E2001]` エラーコード付きでエラー返却

### イベント（追加detail）

```
launch-progress:
  step: "create"
  detail: "Generating branch name..."  // AI生成中のサブステータス
```

```
launch-finished:
  status: "error"
  error: "[E2001] AI branch name generation failed: ..."  // AI失敗時
```
