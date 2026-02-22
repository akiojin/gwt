# 実装計画: Launch Agent From Issue ブランチプレフィックスAI判定

**仕様ID**: `SPEC-a2f8e3b1` | **日付**: 2026-02-20 | **仕様書**: `specs/SPEC-a2f8e3b1/spec.md`

## 目的

- From Issue フローでブランチプレフィックスを「ラベル優先・AIフォールバック」方式で自動判定する
- `bug`/`hotfix` ラベルがない Issue は AI がタイトル・本文・ラベルを分析してプレフィックスを提案する

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-core/` + `crates/gwt-tauri/`）
- **フロントエンド**: Svelte 5 + TypeScript（`gwt-gui/`）
- **AI**: 既存の `AIClient`（`gwt-core::ai::client`）を利用。`ProfilesConfig::resolve_active_ai_settings()` で設定取得
- **テスト**: Rust `#[cfg(test)]` + vitest
- **前提**: `body` フィールドは `fetch_github_issues` で既に取得済み。追加のAPI呼び出し不要

## 実装方針

### Phase 1: バックエンド - AI プレフィックス判定

1. `crates/gwt-core/src/ai/issue_classify.rs` を新規作成
   - 英語のプロンプト定数 `ISSUE_CLASSIFY_SYSTEM_PROMPT` を定義（レスポンスはプレーンテキスト単語1つ）
   - `classify_issue_prefix()` 関数を実装（入力: title, labels, body → 出力: `BranchPrefix`）
   - 本文の切り詰め（先頭 500 文字）はこの関数内で実施
   - レスポンスパーサー `parse_classify_response()` を実装（プレーンテキストから4択の単語を抽出）
   - 4択以外のレスポンスは `Err` を返す
   - タイムアウトは既存の `AIClient` の設定をそのまま利用
2. `crates/gwt-core/src/ai/mod.rs` にモジュールを追加
3. `crates/gwt-tauri/src/commands/issue.rs` に `classify_issue_branch_prefix` Tauri コマンドを追加
   - 入力: `title: String`, `labels: Vec<String>`, `body: Option<String>`
   - 出力: `ClassifyResult { status, prefix, error }`

### Phase 2: フロントエンド - AI 判定の統合

1. `AgentLaunchForm.svelte` の Issue 選択ロジックを修正
   - `determinePrefixForIssue()` 関数を追加（ラベル判定 → AI フォールバック）
   - AI 判定中の状態管理（`prefixClassifying: boolean`）
   - キャッシュ用 `Map<number, BranchPrefix>` を追加
   - リクエスト ID ベースの結果棄却（`classifyRequestId`）
2. プレフィックスドロップダウンの UI 修正
   - 判定中: 空欄 + スピナー表示
   - 失敗時: 空欄（未選択状態）でユーザーが手動選択
3. プレフィル `$effect` を新しい判定フローに統合
4. `BranchPrefix` 型に空文字列を許容する（判定中/失敗時の未選択状態）

### Phase 3: テスト

1. バックエンド単体テスト（`crates/gwt-core/src/ai/issue_classify.rs`）
   - `parse_classify_response()` のパース成功/失敗テスト
   - 不正なプレフィックス値のエラーハンドリング
2. フロントエンド単体テスト（`gwt-gui/src/lib/components/AgentLaunchForm.test.ts`）
   - ラベル判定の優先順位テスト
   - AI 判定中の UI 状態テスト
   - キャッシュ動作テスト

## テスト

### バックエンド

- `parse_classify_response()` が正しいプレーンテキストレスポンスを `BranchPrefix` に変換すること
- 4択以外の値（`fix`, `enhancement` 等）がエラーになること
- 余計なテキスト（説明文等）が含まれていても4択の単語を抽出できること
- 空のレスポンスがエラーになること

### フロントエンド

- `bug` ラベル付き Issue → `bugfix/`、AI 呼び出しなし
- `hotfix` ラベル付き Issue → `hotfix/`、AI 呼び出しなし
- `bug` + `hotfix` 両方 → `hotfix/` 優先
- ラベルなし Issue → AI 判定呼び出し
- AI 判定中のスピナー表示
- AI 失敗時のドロップダウン未選択状態
- Issue 連続切り替え時のリクエスト棄却
- キャッシュからの即時反映
