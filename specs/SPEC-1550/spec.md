### 背景

AI 設定、とくに API キーと model 選択は、ユーザーにとって再起動後も必ず再現されるべき app-wide settings である。保存先が複数あると stale value が優先され、設定画面で保存した値と runtime が参照する値が一致しなくなる。

この仕様では、AI 設定の canonical persistence を **`~/.gwt/config.toml` の profile data** に固定し、#1542 の persistence layout に整合させる。

### 既存コマンド（移植対象）

- `suggest_branch_name`
- `is_ai_configured`
- `list_ai_models`

### AI 機能一覧（全8機能）

現行 gwt が提供する AI 機能の完全リスト:

| # | 機能 | 説明 | 利用箇所 |
|---|------|------|---------|
| 1 | ブランチ名提案 | Issue 内容からブランチ名を自動生成 | worktree 作成時 |
| 2 | Issue 分類 | Issue のブランチプレフィックス（feat/fix/chore）を自動分類 | ブランチ作成時 |
| 3 | セッション要約 | エージェントセッションの自動サマリー生成 | セッション完了時 |
| 4 | コード変更要約 | Git diff から自然言語のサマリーを生成 | ブランチサマリー表示時 |
| 5 | スクロールバック/ターミナル出力要約 | ターミナルの出力内容をAIで要約 | ターミナル出力が大量の場合 |
| 6 | バージョン履歴要約 | タグ間の changelog を自然言語で要約 | バージョン履歴表示時 |
| 7 | ターミナル出力分析 | ターミナル出力からエラー検出・パターン分析 | エージェント作業監視時 |
| 8 | モデル一覧取得 | AI プロバイダーの利用可能モデルを取得 | 設定画面 |

### ユーザーシナリオ

| ID | シナリオ | 優先度 |
|----|---------|--------|
| US-1 | ユーザーが設定画面で endpoint / api_key / model を保存し、再起動後も同じ profile から読める | P0 |
| US-2 | active profile を切り替えると、その profile の AI 設定だけが使われる | P0 |
| US-3 | GET /models は active profile の endpoint / api_key を使って呼ばれる | P1 |
| US-4 | sidecar file を削除しても AI 設定の runtime behavior が変わらない | P1 |
| US-5 | OpenAI 互換 endpoint のみをサポートし、非互換 provider は proxy 経由で扱う | P0 |

### 機能要件

| ID | 要件 | 関連US |
|----|------|--------|
| FR-001 | OpenAI互換HTTP APIのみを使用して全AI機能を実装する | 全US |
| FR-002 | ブランチ名の自動提案をサポートする | US-1 |
| FR-003 | Issue の自動分類をサポートする | US-1 |
| FR-004 | セッションサマリーの自動生成をサポートする | US-3 |
| FR-005 | コード変更サマリーの自動生成をサポートする | US-3 |
| FR-006 | AI 設定の canonical persistence は `~/.gwt/config.toml` の profile data のみとする | US-1, US-2 |
| FR-007 | canonical shape は `[profiles]` に `version`, `active` を持ち、各 profile は `[profiles.<name>]` と `[profiles.<name>.ai]` で表現する | US-1, US-2 |
| FR-008 | `default_ai`, `profiles.profiles.<name>`, `profiles.toml`, `profiles.yaml` などの legacy schema / sidecar file はサポートしない | US-1, US-4 |
| FR-009 | `list_ai_models`, `is_ai_configured`, launch env 注入、summary generation は canonical profile data だけを参照する | US-2, US-3 |
| FR-010 | endpoint 指定後に GET /models でモデル一覧を動的取得・選択できる | US-3 |
| FR-011 | Lead AI のツール定義は C# コード内にハードコードする | US-5 |
| FR-012 | コスト表示 UI は不要とする | - |

### 非機能要件

| ID | 要件 |
|----|------|
| NFR-001 | API key は app-wide settings として `config.toml` の canonical profile data にのみ存在する |
| NFR-002 | 設定画面の保存値と runtime が参照する値が一致する |
| NFR-003 | sidecar file の有無で AI 設定の挙動が変わらない |
| NFR-004 | API key は UI ではマスク表示し、必要時のみコピーできる |

### 成功基準

| ID | 基準 |
|----|------|
| SC-001 | endpoint / api_key / model を保存後、再起動しても active profile から同じ値が読める |
| SC-002 | `profiles.<name>.ai` が AI 設定の唯一の canonical source として文書化されている |
| SC-003 | sidecar file を削除しても AI 機能の挙動が変わらない |
| SC-004 | `list_ai_models` と launch env 注入が canonical profile data と一致する |
