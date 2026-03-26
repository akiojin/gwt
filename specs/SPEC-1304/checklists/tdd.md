### テスト設計

- `gwt-tauri::commands::terminal`
  - `preferred_launch_runner_is_bunx`
  - `build_runner_launch_bunx_uses_package_as_first_arg`
  - `build_runner_launch_npx_uses_yes_flag`
- `gwt-core::terminal::runner` / `terminal::pty`
  - Issue #1265 再発文字列の正規化・batch 判定系回帰
- `gwt-gui`
  - `AgentLaunchForm.test.ts`: `installed` 選択保持の確認
  - `StatusBar.test.ts`: agent 可用性表示を除いた描画/voice 表示確認

### 実行記録

成功:
- `cargo test -p gwt-tauri normalize_launch_command_for_platform -- --test-threads=1`
- `cargo test -p gwt-tauri normalized_process_command -- --test-threads=1`
- `cargo test -p gwt-tauri build_runner_launch -- --test-threads=1`
- `cargo test -p gwt-core terminal::pty -- --test-threads=1`
- `cargo fmt --all -- --check`

未完了（環境要因）:
- `gwt-gui` 側は `pnpm` / `node_modules` 環境不整合により再実行が未完。
