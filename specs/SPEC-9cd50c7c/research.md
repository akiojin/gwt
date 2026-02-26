# リサーチ: AI自動ブランチ命名モード

**仕様ID**: `SPEC-9cd50c7c` | **日付**: 2026-02-26

## 既存実装の調査結果

### 1. AI提案の現行フロー

- フロントエンド: `AgentLaunchForm.svelte` の "Suggest..." ボタン → `generateBranchSuggestions()` → `invoke("suggest_branch_names", { description })`
- バックエンド: `branch_suggest.rs`（Tauri）→ `ProfilesConfig::resolve_active_ai_settings()` → `AIClient::new()` → `gwt_core::ai::suggest_branch_names()`
- AIプロンプト: "Generate exactly 3 branch name suggestions" + JSON形式 `{"suggestions": [...]}`
- パーサー: `parse_branch_suggestions()` → 3つの提案を厳格に検証（プレフィックス4種のみ + サフィックスサニタイズ）

### 2. 削除対象（Suggestモーダル関連）

**状態変数** (AgentLaunchForm.svelte 行132-137):
- `suggestOpen`, `suggestDescription`, `suggestLoading`, `suggestError`, `suggestSuggestions`

**関数**:
- `openSuggestModal()` (行798-802), `closeSuggestModal()` (行803-807)
- `generateBranchSuggestions()` (行809-845)

**UI**: Suggestモーダル全体HTML + "Suggest..." ボタン

### 3. 起動プロセスの既存ステップ

`LaunchProgressModal.svelte` のステップ定義:
1. fetch → Fetching agent info
2. validate → Validating request
3. conflicts → Checking conflicts
4. **create → Creating worktree** ← ここにAI生成を統合
5. deps → Preparing runtime

### 4. LaunchAgentRequest 既存フィールド

- `agentId`, `branch`, `profile`, `model`, `agentVersion`, `mode`, `skipPermissions`
- `createBranch?: { name: string; base?: string | null }`
- `issueNumber?`, `dockerService?`, その他

→ `aiBranchDescription?: string` を追加する

### 5. 永続化スキーマ（agentLaunchDefaults.ts）

- ストレージキー: `gwt.launchAgentDefaults.v1`
- `LaunchDefaults` 型に `branchNamingMode` フィールドを追加する
- 既存フィールドとの競合なし

### 6. AI設定チェックの既存パターン

`branch_suggest.rs` (Tauri) 行37-44:
- `ProfilesConfig::load()` → `resolve_active_ai_settings()` → `None` なら `"ai-not-configured"` を返す
- この同じパターンをフォーム呈示時のチェックに流用可能

### 7. エラーハンドリングの既存パターン

- `LaunchFinishedPayload.status`: "ok" | "cancelled" | "error"
- `LaunchFinishedPayload.error`: エラーメッセージ文字列
- `[E1004]`: ブランチ存在エラー → "Use Existing Branch" ボタン表示
- AI失敗用の新エラーコード（例: `[E2001]`）を導入し、フロントエンドがフォールバック判定に使う

## 要確認事項

なし（clarifyで全て解消済み）
