> **ℹ️ TUI MIGRATION NOTE**: This SPEC was completed during the gwt-tauri era. The gwt-tauri frontend has been replaced by gwt-tui (SPEC-1776). GUI-specific references are historical.

### Background
Settings > Profiles の API Key 入力で、手入力した値は一時的に使えても Save 後に reopen すると失われる。また、貼り付けた値は UI state に反映されず、Peek/Copy ボタン表示、Refresh、Save に使えない。関連 bug report: #1480。

### User Scenarios
- P0: ユーザーが Profiles タブで API Key を手入力して Save すると、設定を閉じて再度開いても同じ値が保持される。
- P0: ユーザーが API Key を貼り付けると、Peek/Copy ボタンが表示され、Refresh が貼り付けた値で `list_ai_models` を呼ぶ。
- P0: ユーザーが貼り付けた API Key を Save すると、設定を閉じて再度開いても同じ値が保持される。
- P1: `default` プロファイルで上記の挙動が安定して動作する。

### Functional Requirements
- FR-001: API Key input は手入力と貼り付けの両方で `apiKeyDraft` と profile state を同期する。
- FR-002: API Key が非空になった時点で Peek/Copy ボタンを表示し、空になった時点で非表示にする。
- FR-003: Refresh は直近の入力値または貼り付け値を `list_ai_models` の `apiKey` 引数に渡す。
- FR-004: Save は Svelte reactive proxy を IPC に渡さず、plain data の `ProfilesConfig` と `SettingsData` を保存する。
- FR-005: `default` プロファイルの `ai.api_key` は Save 後の reopen/reload でも保持される。

### Non-Functional Requirements
- NFR-001: 既存の手入力ワークフローを壊さない。
- NFR-002: 公開 API、config schema、Tauri command 名を変更しない。
- NFR-003: Unit test と Playwright E2E の両方で再現ケースをカバーする。

### Success Criteria
- SC-001: `SettingsPanel.test.ts` に手入力・貼り付け・Save・reopen の回帰テストが追加され、すべて通る。
- SC-002: `settings-config.spec.ts` に paste 経路の E2E が追加され、貼り付け後の表示と invoke 引数を確認できる。
- SC-003: `ProfilesConfig` の roundtrip test で `default.ai.api_key` 非空値が保持される。
