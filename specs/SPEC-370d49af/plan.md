### 変更ファイル一覧

| # | ファイル | 変更内容 |
|---|---------|---------|
| 1 | `crates/gwt-tauri/src/commands/terminal.rs` | 5 つの match 関数に copilot アームを追加 |
| 2 | `crates/gwt-tauri/src/commands/agents.rs` | `detect_copilot()` 関数追加 + detect_agents vec に登録 |
| 3 | `gwt-gui/src/lib/agentUtils.ts` | `AgentId` 型 + `inferAgentId` に copilot 追加 |
| 4 | `gwt-gui/src/lib/components/agentLaunchFormHelpers.ts` | `supportsModelFor()` に copilot 追加 |
| 5 | `gwt-gui/src/lib/components/AgentLaunchForm.svelte` | modelOptions に copilot 用モデル一覧追加 |

### 実装アプローチ

既存 4 エージェント（Claude Code / Codex / Gemini / OpenCode）と同一パターンに従い、各 match/if 分岐に copilot アームを追加する最小変更アプローチ。
