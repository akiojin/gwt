### Phase 1: gpt-5.4 モデル追加（完了）

1. Codex 新モデル対応の親仕様として本 Issue を維持し、今後の追記先を固定する
2. `gpt-5.4` 追加に関する GUI / core の RED テストを先に更新する
3. Codex モデル一覧を `gpt-5.4` 対応へ更新し、既定モデルは version gate 付きで切り替える
4. Codex の実効モデルが `gpt-5.4` の場合だけ 1M context 用 `-c` 引数を追加する
5. 対象テストと型チェックを実行して GREEN を確認する
6. 本 Issue を更新する

### Phase 2: Fast mode 対応（完了）

1. Fast mode 関連の RED テストを先に追加する（Rust + Frontend）
2. `LaunchAgentRequest` に `fastMode` フィールドを追加する（Frontend types.ts + Rust struct）
3. `codex_default_args()` に `fast_mode: bool` パラメータを追加し、条件付きで `-c service_tier=fast` を付与する
4. `AgentLaunchForm.svelte` に Fast mode チェックボックスを追加する（gpt-5.4 選択時のみ表示）
5. `agentLaunchDefaults` に `fastMode` フィールドを追加し、保存/復元を実装する
6. 対象テスト、型チェック、format、clippy を実行して GREEN を確認する

### Phase 3: multi-agent 親仕様化と Codex モデル一覧更新（完了）

1. 本 Issue のタイトルと `## Spec` を、Codex 専用から multi-agent のモデル管理仕様へ更新する
2. `AgentLaunchForm.test.ts` の Codex モデル一覧期待値を test-first で `gpt-5.4-mini` を含む最新順序へ更新し、GUI テストで固定する
3. `AgentLaunchForm.svelte` の Codex モデル一覧を最新順序へ更新する
4. 既存の defaults 復元・Codex 引数関連の回帰テストを実行し、既存挙動が維持されることを確認する
5. 本 Issue の Tasks / TDD / Acceptance Checklist を実装結果に合わせて更新する
