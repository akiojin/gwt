### Phase 1: gpt-5.4 モデル追加（完了）

- [x] T001 [S] Codex 新モデル対応の親 `gwt-spec` Issue を作成し、今後の更新先を固定する
- [x] T002 [U] `AgentLaunchForm.test.ts` の Codex モデル一覧期待値を `gpt-5.4` 含みに更新して RED を確認する
- [x] T003 [U] `AgentLaunchForm.test.ts` の選択モデル送信テストを `gpt-5.4` 前提に更新して RED を確認する
- [x] T004 [U] `crates/gwt-core/src/agent/codex.rs` の既定モデルテストを `gpt-5.4` 期待に更新して RED を確認する
- [x] T005 [U] Codex モデル一覧へ `gpt-5.4` を追加し、並び順を更新する
- [x] T006 [U] Codex の既定モデルを version gate 付きで `latest` と `0.111.0+`=`gpt-5.4` / 古い resolved version=`gpt-5.2-codex` に更新する
- [x] T007 [U] 保存済み defaults 関連テストを確認し、後方互換のため既存モデル保持を継続する
- [x] T008 [FIN] Rust / GUI テストと `svelte-check` / `cargo fmt --check` を実行して GREEN を確認する
- [x] T009 [FIN] 本 Issue を更新する
- [x] T010 [U] `gpt-5.4` 実効時の context override を要求する RED テストを `gwt-core` に追加する
- [x] T011 [U] `build_agent_args` で context override が反映される RED テストを `gwt-tauri` に追加する
- [x] T012 [U] Codex の実効モデルが `gpt-5.4` の場合だけ `model_context_window=1000000` / `model_auto_compact_token_limit=950000` を付与する
- [x] T013 [FIN] `gwt-core` / `gwt-tauri` の対象テストと `cargo fmt --check` を再実行して GREEN を確認する
- [x] T014 [FIN] 本 Issue の FR / SC / TDD / Research を context override 仕様へ更新する

### Phase 2: Fast mode 対応（完了）

- [x] T015 [U] [FR-009,FR-010] `codex.rs` に fast_mode=true で `-c service_tier=fast` が含まれる RED テストを追加する
- [x] T016 [U] [FR-011] `codex.rs` に fast_mode=false で `service_tier` が含まれない RED テストを追加する
- [x] T017 [U] [FR-010] `terminal.rs` に fast_mode 付き `build_agent_args` の RED テストを追加する
- [x] T018 [U] [FR-009] `AgentLaunchForm.test.ts` に gpt-5.4 選択時の Fast mode チェックボックス表示テストを追加する
- [x] T019 [U] [FR-012] `agentLaunchDefaults.test.ts` に fastMode の保存/復元テストを追加する
- [x] T020 [U] [FR-009-012] `codex_default_args()` に `fast_mode` パラメータを追加し、`-c service_tier=fast` の条件付き付与を実装する
- [x] T021 [U] [FR-009-012] `LaunchAgentRequest` に `fastMode` フィールドを追加し（types.ts + Rust struct）、`build_agent_args()` から `codex_default_args()` へ渡す
- [x] T022 [U] [FR-009,FR-012] `AgentLaunchForm.svelte` に Fast mode チェックボックスを追加する（gpt-5.4 選択時のみ表示、defaults 保存/復元対応）
- [x] T023 [U] [FR-012] `agentLaunchDefaults.ts` に `fastMode` フィールドを追加する
- [x] T024 [FIN] Rust / GUI テストと `svelte-check` / `cargo fmt --check` / `cargo clippy` を実行して GREEN を確認する
- [x] T025 [FIN] 本 Issue のチェックリストを更新する

### Phase 3: multi-agent 親仕様化と Codex モデル一覧更新（完了）

- [x] T026 [S] 本 Issue のタイトル・Spec・Plan・Tasks を multi-agent のモデル管理親仕様へ更新する
- [x] T027 [U] `AgentLaunchForm.test.ts` の Codex モデル一覧期待値を test-first で `gpt-5.4-mini` を含む最新順序へ更新する
- [x] T028 [U] `AgentLaunchForm.svelte` の Codex モデル一覧を最新順序へ更新する
- [x] T029 [FIN] `pnpm test` の対象テストと Codex 関連 Rust テストを再実行して GREEN を確認する
- [x] T030 [FIN] 本 Issue の Acceptance Checklist と Research を実装結果に合わせて更新する
