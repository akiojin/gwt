# データモデル: AI自動ブランチ命名モード

**仕様ID**: `SPEC-9cd50c7c` | **日付**: 2026-02-26

## バックエンド（Rust）

### 変更: BranchSuggestResult（gwt-tauri）

```
Before:
  status: String          // "ok" | "ai-not-configured" | "error"
  suggestions: Vec<String> // 3つのフルネーム
  error: Option<String>

After:
  status: String          // "ok" | "ai-not-configured" | "error"
  suggestion: String      // 1つのフルネーム（"feature/add-login"）
  error: Option<String>
```

### 変更: LaunchAgentRequest（gwt-tauri）

```
追加フィールド:
  ai_branch_description: Option<String>  // AI Suggestモード時の説明文
```

- `ai_branch_description` が `Some` かつ `create_branch` がある場合: AI生成フロー
- `ai_branch_description` が `None` の場合: 従来フロー

### 変更: BranchSuggestionsResponse（gwt-core）

```
Before:
  suggestions: Vec<String>

After:
  suggestion: String
```

### 新規: エラーコード

- `[E2001]`: AIブランチ名生成失敗（`launch_agent_for_project_root` 内で使用）

## フロントエンド（TypeScript）

### 変更: BranchSuggestResult

```
Before:
  suggestions: string[]

After:
  suggestion: string
```

### 変更: LaunchAgentRequest

```
追加フィールド:
  aiBranchDescription?: string
```

### 変更: LaunchDefaults（localStorage）

```
追加フィールド:
  branchNamingMode: "direct" | "ai-suggest"  // デフォルト: "ai-suggest"
```

## AIプロンプト変更

```
Before: "Generate exactly 3 branch name suggestions..."
         JSON: {"suggestions": ["prefix/name-1", "prefix/name-2", "prefix/name-3"]}

After:  "Generate exactly 1 branch name suggestion..."
         JSON: {"suggestion": "prefix/name"}
```
