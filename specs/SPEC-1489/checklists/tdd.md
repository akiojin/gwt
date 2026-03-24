### Phase 1: gpt-5.4 モデル追加（完了）
- T-001: Codex モデル一覧テストに `gpt-5.4` と新しい並び順を要求する
- T-002: Codex モデル送信テストで `gpt-5.4` が launch request に入ることを要求する
- T-003: Codex 既定引数テストで `latest`、`0.111.0+`、古い resolved version の model gate を要求する
- T-004: Codex 既定引数テストで、実効モデルが `gpt-5.4` のとき context override が付くことを要求する
- T-005: Codex 既定引数テストで、`gpt-5.4` 以外では context override が付かないことを要求する
- T-006: `build_agent_args` テストで、`gpt-5.4` 明示選択と `latest` 既定解決の双方で context override が出ることを要求する

### Phase 2: Fast mode 対応（完了）
- T-007: `codex_default_args` テストで fast_mode=true のとき `-c service_tier=fast` が含まれることを要求する
- T-008: `codex_default_args` テストで fast_mode=false のとき `service_tier` が含まれないことを要求する
- T-009: `build_agent_args` テストで fast_mode=true が `service_tier=fast` として反映されることを要求する
- T-010-FM: `AgentLaunchForm` テストで gpt-5.4 選択時に Fast mode チェックボックスが表示されることを要求する
- T-011-FM: `AgentLaunchForm` テストで gpt-5.4 以外のモデル選択時に Fast mode チェックボックスが表示されないことを要求する
- T-012-FM: `agentLaunchDefaults` テストで fastMode の保存/復元が正しく動作することを要求する

### Phase 3: multi-agent 親仕様化と Codex モデル一覧更新（完了）
- T-013: `AgentLaunchForm.test.ts` で Codex モデル一覧が `gpt-5.4-mini` を含む 8 件の最新順序になることを要求する
- T-014: 既存の defaults 保存/復元テストで、model ID の後方互換が維持されることを回帰確認する
- T-015: `codex_default_args` / `build_agent_args` の既存テストで、既定モデル gate と `gpt-5.4` 固有挙動が維持されることを回帰確認する

### 既存テストで回帰確認
- T-020: `agentLaunchDefaults.test.ts` の保存/復元テスト
- T-021: `AgentLaunchForm.test.ts` の既存 Launch / Reasoning 系テスト
- T-022: `build_agent_args` の Codex 起動引数テスト
- T-023: `gwt-core` の Codex 既定引数テスト
