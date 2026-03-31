### Rust テスト（terminal.rs #[cfg(test)]）

- `builtin_agent_def_copilot`: copilot の定義が正しいことを検証
- `build_agent_model_args_copilot`: `--model` 引数生成を検証
- `agent_color_for_copilot`: Blue 色を検証
- `tool_id_for_copilot`: `github-copilot` ID を検証
- `build_agent_args_copilot_continue`: Continue モードで `--continue` が含まれることを検証
- `build_agent_args_copilot_skip_permissions`: `--allow-all-tools` が含まれることを検証

### フロントエンドテスト

- `agentLaunchFormHelpers.test.ts`: `supportsModelFor("copilot")` が true
- `agentUtils.test.ts`: `inferAgentId("copilot")` と `inferAgentId("GitHub Copilot")` が `"copilot"`
