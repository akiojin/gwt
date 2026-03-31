> **ℹ️ TUI MIGRATION NOTE**: This SPEC was completed during the gwt-tauri era. The gwt-tauri frontend has been replaced by gwt-tui (SPEC-1776). GUI-specific references are historical.

### 背景

Settings > Profiles で `ai.api_key` を保存しても、macOS 環境で Codex が `Not authenticated` 扱いになる。
同一 API キーを Windows 側で使うと認証済みに見えるケースがあり、OS 間で挙動が不一致。

原因調査により、Codex 認証判定と起動環境が `OPENAI_API_KEY` 環境変数のみを参照し、
`profiles.toml` の `active profile.ai.api_key` を参照していないことが確認された。

### ユーザーシナリオとテスト（受け入れシナリオ）

**US-1: 設定保存した API キーで認証表示が一致する** [P0]

- 前提: Active Profile の `ai.api_key` に有効なキーが保存済み、`OPENAI_API_KEY` は未設定
- 操作: Agent 検出を実行する
- 期待: Codex が `authenticated=true` と判定される

**US-2: 起動時に API キーが環境へ注入される** [P0]

- 前提: Active Profile の `ai.api_key` が非空、`profile.env.OPENAI_API_KEY` は未設定
- 操作: Codex を Launch する
- 期待: 起動環境に `OPENAI_API_KEY=<ai.api_key>` が含まれる

**US-3: 明示 env は優先される** [P0]

- 前提: `profile.env.OPENAI_API_KEY` が設定済み、`ai.api_key` も設定済み
- 操作: Codex を Launch する
- 期待: `profile.env.OPENAI_API_KEY` が優先される（既存優先順位を維持）

### 機能要件

| ID | 要件 |
|----|------|
| FR-001 | Codex 認証判定は `OPENAI_API_KEY` 環境変数 **または** Active Profile の `ai.api_key` のいずれか非空で true とする |
| FR-002 | `detect_agents` の installed/fallback 双方で FR-001 と同じ判定ロジックを用いる |
| FR-003 | Codex Launch 時、`OPENAI_API_KEY` が未設定なら Active Profile の `ai.api_key` を環境変数へ注入する |
| FR-004 | `profile.env.OPENAI_API_KEY` や request overrides など既存の優先順位を変更しない |
| FR-005 | API キー文字列は trim 判定し、空白のみは未設定扱いにする |

### 非機能要件

| ID | 要件 |
|----|------|
| NFR-001 | 変更は `gwt-core` / `gwt-tauri` の認証判定と env 注入の最小範囲に限定する |
| NFR-002 | 回帰防止として Rust テストと GUI E2E シナリオを追加する |

### 成功基準

| ID | 基準 |
|----|------|
| SC-001 | `OPENAI_API_KEY` 未設定でも `ai.api_key` 設定時に Codex 認証判定テストが GREEN |
| SC-002 | Launch 用環境変数生成テストで `OPENAI_API_KEY` 注入が確認できる |
| SC-003 | 既存優先順位（env > ai.api_key）が維持されるテストが GREEN |
| SC-004 | Playwright E2E に API キー保存シナリオを追加し GREEN |
