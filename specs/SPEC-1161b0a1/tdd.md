# TDDノート: Windows 移行プロジェクトの Docker 起動ポート競合回避

## RED

- 追加テスト:
  - `commands::terminal::tests::merge_compose_env_for_docker_keeps_existing_allocated_port_when_incoming_is_occupied`
- 実行:
  - `cargo test -p gwt-tauri merge_compose_env_for_docker_keeps_existing_allocated_port_when_incoming_is_occupied`
- 結果:
  - 失敗（incoming の使用中ポート値で上書きされる）

## GREEN

- 修正:
  - `merge_compose_env_for_docker` に、既存値/新規値がポート値かつ新規ポートが使用中の場合は既存値を保持するガードを追加
- 実行:
  - `cargo test -p gwt-tauri merge_compose_env_for_docker`
- 結果:
  - 2 tests passed
