<!-- markdownlint-disable MD013 -->
# フェーズ0: 調査レポート

**仕様ID**: `SPEC-1ad9c07d` | **日付**: 2026-02-08

## 1. 既存コードベース分析

### 技術スタック

- **言語**: Rust 2021 Edition (stable)
- **TUI**: ratatui 0.29 + crossterm 0.28
- **HTTP**: reqwest (blocking)
- **シリアライズ**: serde, serde_json
- **テスト**: cargo test
- **AI API**: OpenAI互換 Responses API (`/responses` エンドポイント)

### ウィザードアーキテクチャ (`wizard.rs`)

ウィザードは中央ポップアップとして表示され、`WizardStep` enumと `WizardState` structで状態管理される。

**現在のステップフロー**:

```text
WizardStep enum:
  QuickStart → BranchAction → BranchTypeSelect → IssueSelect → BranchNameInput
  → AgentSelect → ModelSelect → ReasoningLevel → VersionSelect
  → CollaborationModes → ExecutionMode → ConvertAgentSelect
  → ConvertSessionSelect → SkipPermissions
```

**ステップ遷移パターン**:

- `next_step()`: matchでステップごとに次ステップを返す。条件分岐（スキップ）もここで処理
- `prev_step()`: matchで前ステップを返す。最初のステップではwizardをclose
- 各ステップには `select_up()`/`select_down()`/`insert_char()`/`delete_char()` のハンドラあり

**レンダリングパターン**:

- `render_wizard_popup()` がポップアップ枠を描画
- ステップごとの `render_*_step()` 関数がコンテンツを描画
- フッターにキー操作のヒントを表示

### AI Client (`gwt-core/src/ai/client.rs`)

- `AIClient::new(settings: ResolvedAISettings)` でクライアント生成
- `create_response(messages: Vec<ChatMessage>) -> Result<String, AIError>` でAPI呼び出し
- `ChatMessage { role: String, content: String }` で入力メッセージ構築
- system roleメッセージは `instructions` フィールドに抽出される
- MAX_OUTPUT_TOKENS: 400, TEMPERATURE: 0.3
- 自動リトライ: RateLimited/ServerErrorで最大5回（指数バックオフ）

### 非同期パターン (`app/ai_wizard.rs`)

```text
1. mpsc::channel() でチャネル作成
2. thread::spawn() でブロッキングAPI呼び出しを別スレッドで実行
3. tx.send() で結果を送信
4. apply_*_updates() で rx.try_recv() により非ブロッキングで結果を受信
5. TryRecvError::Empty → チャネルを再設定して待機継続
6. TryRecvError::Disconnected → チャネル破棄
```

### ブランチ名正規化 (`gwt-core/src/agent/worktree.rs`)

`sanitize_branch_name(name: &str) -> String`:

- 小文字化
- スペース/アンダースコア → ハイフン
- 非英数字・非ハイフン文字を除去
- 連続ハイフンを1つに圧縮
- 先頭/末尾ハイフンをトリム
- 64文字に切り詰め

### AI設定アクセス (`config/profile.rs` + `app.rs`)

- `AISettings { endpoint, api_key, model, summary_enabled }` - 設定構造体
- `is_enabled()`: endpoint.trim() != "" AND model.trim() != ""
- `ResolvedAISettings { endpoint, api_key, model }` - トリム済み設定
- `app.rs` の `active_ai_enabled()` / `active_ai_settings()` メソッドでアクセス

### BranchType (`wizard.rs`)

```rust
BranchType::Feature  → "feature/"
BranchType::Bugfix   → "bugfix/"
BranchType::Hotfix   → "hotfix/"
BranchType::Release  → "release/"
```

`full_branch_name()`: `format!("{}{}", self.branch_type.prefix(), self.new_branch_name)`

## 2. 技術的決定

### 決定1: AIBranchSuggestの状態管理

**方針**: WizardState内にインラインで新フィールドを追加する（別構造体に分離しない）。

**理由**: 既存パターン（IssueSelectのフィールド群など）と一貫性を保つ。

### 決定2: 非同期AI呼び出しパターン

**方針**: ai_wizard.rsの既存パターン（thread::spawn + mpsc::channel + try_recv）を踏襲する。

**理由**: プロジェクト内で確立されたパターンであり、新たな依存関係を追加しない。

### 決定3: AI設定の取得方法

**方針**: WizardState自体にはAI設定を持たせず、app.rs側の `active_ai_enabled()` と `active_ai_settings()` を使って遷移時にチェックする。

**理由**: WizardStateは設定へのアクセスを持たないため、app.rs側の遷移ロジック（`handle_wizard_enter()`相当）でAI有効/無効を判定し、AIBranchSuggestをスキップするかどうかを決定する。ただし、`next_step()` はWizardState内のメソッドであるため、WizardStateにAI有効フラグを持たせる方式が最もシンプル。

**最終決定**: `WizardState`に `ai_enabled: bool` フラグを追加し、wizard open時にapp.rsから `active_ai_enabled()` の結果を設定する。

### 決定4: プレフィックスからBranchTypeへの変換

**方針**: 候補ブランチ名からプレフィックスを抽出し、対応するBranchTypeに変換する関数を追加する。

**ロジック**:

1. 候補が `feature/`, `bugfix/`, `hotfix/`, `release/` で始まるかチェック
2. マッチしたらそのBranchTypeを返し、プレフィックス後の部分を名前として返す
3. マッチしなければ現在選択中のBranchTypeを維持し、候補全体を名前として返す

### 決定5: AIプロンプト設計

**方針**: システムメッセージでJSON出力形式を指定し、ユーザーメッセージでブランチ目的と選択済みタイプを伝える。

**レスポンス形式**: `{"suggestions": ["feature/add-login-page", "feature/implement-oauth", "feature/login-integration"]}`

## 3. 制約と依存関係

### 制約1: UIはブロッキング不可

AIリクエストはバックグラウンドスレッドで実行し、UIスレッドをブロックしない。ローディング中もEscによるキャンセルを受け付ける。

### 制約2: 既存フローへの非侵入性

AI無効時は従来フロー（IssueSelect → BranchNameInput）を完全に維持する。AIBranchSuggestステップは条件付きスキップとして実装する。

### 制約3: CLIテキストは英語のみ

ユーザー向けのラベル、プロンプト、エラーメッセージはすべて英語で記述する。

### 制約4: 既存テストの維持

既存のwizardテストが壊れないことを保証する。新しいenumバリアントの追加により、exhaustiveなmatchが必要になる箇所を網羅的に更新する。

## 4. 解消済み「要確認」項目

- AI設定へのアクセス方法 → WizardStateに `ai_enabled` フラグを追加、open時に設定
- 非同期パターン → 既存ai_wizard.rsパターンを踏襲
- プレフィックス変換ロジック → BranchType::from_prefix()メソッドを追加
- エラー時のフォールバック → エラーフェーズを追加し、Enterで手動入力へ遷移
