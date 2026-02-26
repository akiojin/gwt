# 実装計画: AI自動ブランチ命名モード

**仕様ID**: `SPEC-9cd50c7c` | **日付**: 2026-02-26 | **仕様書**: `specs/SPEC-9cd50c7c/spec.md`

## 目的

- ブランチ命名のAI提案を「3候補から選択」式から「1つ自動生成」式に変更し、Launch時の操作ステップを削減する
- Direct / AI Suggest のセグメンテッドボタンで切り替えるUIに刷新する
- 既存のSuggestモーダルを廃止し、AIブランチ命名をworktreeステップ内で非同期実行する

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-core/src/ai/branch_suggest.rs`, `crates/gwt-tauri/src/commands/branch_suggest.rs`, `crates/gwt-tauri/src/commands/terminal.rs`）
- **フロントエンド**: Svelte 5 + TypeScript（`gwt-gui/src/lib/components/AgentLaunchForm.svelte`, `gwt-gui/src/lib/components/LaunchProgressModal.svelte`）
- **ストレージ**: localStorage（`gwt-gui/src/lib/agentLaunchDefaults.ts` キー: `gwt.launchAgentDefaults.v1`）
- **テスト**: cargo test（Rust unit tests）/ vitest（フロントエンド）
- **前提**: AI設定は `ProfilesConfig::resolve_active_ai_settings()` で取得。セグメンテッドボタンは既存のmanual/fromIssueタブと同様のデザイン

## 実装方針

### Phase 1: バックエンド改修（Rust コア + Tauri）

#### 1-1. AIプロンプトとパーサーの1つ生成化

**対象**: `crates/gwt-core/src/ai/branch_suggest.rs`

- `BRANCH_SUGGEST_SYSTEM_PROMPT` を変更: "Generate exactly 3" → "Generate exactly 1"
- JSONフォーマットを変更: `{"suggestions": [...]}` → `{"suggestion": "prefix/name"}`
- `BranchSuggestionsResponse` のフィールドを `suggestions: Vec<String>` → `suggestion: String` に変更
- `parse_branch_suggestions()` を `parse_branch_suggestion()` にリネーム:
  - 1つの提案のみパース・検証
  - プレフィックス検証（4種）+ サフィックスサニタイズは維持
  - 戻り値: `Result<String, AIError>`
- `suggest_branch_names()` を `suggest_branch_name()` にリネーム:
  - 戻り値: `Result<String, AIError>`
- テストケースを更新（3→1の検証に変更）

#### 1-2. Tauriコマンドの改修

**対象**: `crates/gwt-tauri/src/commands/branch_suggest.rs`

- `BranchSuggestResult` の `suggestions: Vec<String>` → `suggestion: String` に変更
- `suggest_branch_names` コマンド → `suggest_branch_name` にリネーム
- 内部呼び出しを `gwt_core::ai::suggest_branch_name()` に変更

#### 1-3. launch_terminal内にAIブランチ名生成を統合

**対象**: `crates/gwt-tauri/src/commands/terminal.rs`

- `LaunchAgentRequest` に `ai_branch_description: Option<String>` フィールド追加
- 行3661-3690の "create" ステップ内を改修:
  - `ai_branch_description` が `Some` の場合:
    1. `report_launch_progress(job_id, &app_handle, "create", Some("Generating branch name..."))`
    2. `ProfilesConfig::load()` → `resolve_active_ai_settings()` → `AIClient::new()`
    3. `suggest_branch_name(&client, &description)` を呼び出し
    4. 生成されたブランチ名で `create_new_worktree_path()` を呼ぶ
    5. AI失敗時: `StructuredError` にエラーコード `[E2001]` を含めて返却
  - `ai_branch_description` が `None` の場合: 既存のフロー（`createBranch.name` を使用）

### Phase 2: フロントエンド改修（Svelte + TypeScript）

#### 2-1. 型定義の更新

**対象**: `gwt-gui/src/lib/types.ts`

- `BranchSuggestResult`: `suggestions: string[]` → `suggestion: string` に変更
- `LaunchAgentRequest`: `aiBranchDescription?: string` フィールド追加

#### 2-2. 永続化スキーマの拡張

**対象**: `gwt-gui/src/lib/agentLaunchDefaults.ts`

- `LaunchDefaults` 型に `branchNamingMode: "direct" | "ai-suggest"` 追加
- デフォルト値: `"ai-suggest"`
- `saveLaunchDefaults()` / `loadLaunchDefaults()` は既存パターンで自動対応

#### 2-3. AgentLaunchForm UIの刷新

**対象**: `gwt-gui/src/lib/components/AgentLaunchForm.svelte`

**削除:**
- Suggestモーダル関連の状態変数（行132-137）: `suggestOpen`, `suggestDescription`, `suggestLoading`, `suggestError`, `suggestSuggestions`
- Suggestモーダル制御関数: `openSuggestModal()`, `closeSuggestModal()`, `generateBranchSuggestions()`
- SuggestモーダルHTML全体 + "Suggest..." ボタン

**追加:**
- 状態変数:
  - `branchNamingMode: "direct" | "ai-suggest"` — セグメンテッドボタンの選択状態
  - `aiDescription: string` — AI Suggestモードの説明入力値
  - `aiConfigured: boolean` — AI設定の有無（フォーム呈示時にチェック）
  - `aiFallbackError: string | null` — フォールバック時のエラーバナー
- セグメンテッドボタン（manualタブ内、newBranchTab === "manual" 時のみ表示）:
  - "Direct" / "AI Suggest" の2択
  - AI未設定時: "AI Suggest" セグメントを `disabled` + ツールチップ
- AI Suggestモード表示:
  - 「Description」ラベル + 単行テキストフィールド（placeholder: "e.g. Add user authentication feature"）
  - baseBranch選択（従来と同じ）
- Directモード表示:
  - 従来のPrefix選択 + Suffix入力（Suggest...ボタンなし）
  - baseBranch選択（従来と同じ）

#### 2-4. Launch処理の改修

**対象**: `gwt-gui/src/lib/components/AgentLaunchForm.svelte`

- `handleLaunch()` 内の request 構築:
  - AI Suggestモード時: 先に `suggest_branch_name` を呼んでブランチ名を確定し、`request.branch` と `createBranch.name` の両方に同じ確定名をセットして launch する
  - Directモード時: 従来通り `createBranch.name` にフルネームをセット
- Launchボタンのdisabled条件を更新:
  - AI Suggestモード: `!aiDescription.trim()` の場合disabled
  - Directモード: 従来通り `!newBranchFullName.trim()` の場合disabled
- AI設定チェック: フォーム初期化時に `invoke("is_ai_configured")` を利用（推論呼び出しは行わない）

#### 2-5. フォールバック処理の実装

**対象**: `gwt-gui/src/lib/components/AgentLaunchForm.svelte`

- `launch-finished` イベントのエラーハンドリング（親コンポーネントまたは本コンポーネント）:
  - `error` に `[E2001]` が含まれる場合:
    1. `branchNamingMode` を `"direct"` に切替
    2. `aiFallbackError` を "AI suggestion failed. Please enter branch name manually." にセット
    3. `aiDescription` は内部保持（クリアしない）
  - バナーの自動消去: `branchNamingMode` が変更されたら `aiFallbackError = null`

### Phase 3: テスト・クリーンアップ

#### 3-1. Rust unit tests

- `parse_branch_suggestion()`: 正常JSON → ブランチ名1つ返却
- `parse_branch_suggestion()`: 不正なprefix → エラー
- `parse_branch_suggestion()`: suffix空 → エラー
- `parse_branch_suggestion()`: sanitization適用確認（大文字→小文字、特殊文字→ハイフン）
- `suggest_branch_name()`: AI応答1つ → String返却

#### 3-2. フロントエンドvitest

- セグメンテッドボタン: Direct選択 → Prefix+Suffix表示、AI Suggest非表示
- セグメンテッドボタン: AI Suggest選択 → Description表示、Prefix+Suffix非表示
- AI未設定: AI Suggestセグメントdisabled
- AI Suggest + Description空: Launchボタンdisabled
- AI Suggest + Description入力: Launchボタンenabled
- モード永続化: localStorageに保存・復元
- フォールバック: エラーバナー表示 + Directモード切替
- Description値保持: モード切替後もaiDescriptionが内部保持される

#### 3-3. コードクリーンアップ

- Suggestモーダル関連のデッドコード確認
- 未使用インポート・型定義の削除
- `cargo clippy` + `cargo fmt` + `svelte-check` パス確認

## リスク・依存関係

| リスク | 影響度 | 対策 |
|--------|--------|------|
| AI応答がworktreeステップを大幅に遅延させる | 中 | 既存AIClientのタイムアウトに依存。フォールバック処理でカバー |
| AI生成ブランチ名がremote-firstフローで既存ブランチと衝突 | 低 | 既存の[E1004]エラーハンドリングがそのまま機能する |
| agentLaunchDefaults のスキーマ変更で既存保存データが壊れる | 低 | loadLaunchDefaults()の既存のnullフォールバックで吸収 |

## テスト

### バックエンド

- `parse_branch_suggestion()`: 正常系（1つの完全なブランチ名）
- `parse_branch_suggestion()`: 異常系（不正prefix、空suffix、パース不能）
- `parse_branch_suggestion()`: sanitization適用確認
- `suggest_branch_name()`: AIClient呼び出し→String返却
- launch_terminal: `ai_branch_description` あり → AI生成 → worktree作成
- launch_terminal: AI失敗 → `[E2001]` エラー返却

### フロントエンド

- セグメンテッドボタンの表示切替
- AI未設定時のdisabled状態
- バリデーション（Description空→Launch disabled）
- モード永続化（保存・復元・AI未設定時の降格）
- フォールバック（エラーバナー表示・モード切替・バナー自動消去）
- Description値の内部保持
