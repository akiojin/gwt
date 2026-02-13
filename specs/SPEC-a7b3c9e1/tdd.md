# TDD計画: エージェントタブ MCP通信ブリッジ

## テスト戦略

本機能は3層に分かれるため、各層ごとにテストを設計する。

1. **gwt-core**: 設定ファイル操作（ユニットテスト）
2. **gwt-tauri**: WebSocketサーバー + MCPツールハンドラー（結合テスト）
3. **gwt-mcp-bridge**: MCPプロトコル + WebSocket通信（E2Eテスト）

## 1. gwt-core ユニットテスト

### 1.1 mcp_registration: JSON設定ファイル操作

```text
テストファイル: crates/gwt-core/src/config/mcp_registration.rs (#[cfg(test)] mod tests)

テストケース:
- test_read_claude_config_empty
  入力: 空の~/.claude.json（ファイル不存在）
  期待: 空のmcpServersで新規作成

- test_read_claude_config_existing
  入力: 既存のmcpServersに他のサーバーがある~/.claude.json
  期待: 既存設定を保持したまま読み込み

- test_register_claude_mcp_server
  入力: gwt-agent-bridge未登録の~/.claude.json
  操作: register_mcp_server("claude", bridge_config)
  期待: mcpServersにgwt-agent-bridgeが追加、他のサーバーは保持

- test_unregister_claude_mcp_server
  入力: gwt-agent-bridge登録済みの~/.claude.json
  操作: unregister_mcp_server("claude")
  期待: gwt-agent-bridgeのみ削除、他のサーバーは保持

- test_register_gemini_mcp_server
  入力: 既存の~/.gemini/settings.json
  操作: register_mcp_server("gemini", bridge_config)
  期待: mcpServersにgwt-agent-bridgeが追加
```

### 1.2 mcp_registration: TOML設定ファイル操作

```text
テストケース:
- test_read_codex_config_empty
  入力: 空の~/.codex/config.toml（ファイル不存在）
  期待: 空の設定で新規作成

- test_read_codex_config_existing
  入力: 既存のmcp_serversセクションがあるconfig.toml
  期待: 既存設定を保持したまま読み込み

- test_register_codex_mcp_server
  入力: gwt-agent-bridge未登録のconfig.toml
  操作: register_mcp_server("codex", bridge_config)
  期待: [mcp_servers.gwt-agent-bridge]セクションが追加

- test_unregister_codex_mcp_server
  入力: gwt-agent-bridge登録済みのconfig.toml
  操作: unregister_mcp_server("codex")
  期待: gwt-agent-bridgeセクションのみ削除
```

### 1.3 mcp_registration: クリーンアップ

```text
テストケース:
- test_cleanup_stale_registration
  入力: gwt-agent-bridgeが残存している全エージェント設定
  操作: cleanup_stale_registrations()
  期待: 全エージェントの設定からgwt-agent-bridgeが削除

- test_cleanup_no_stale
  入力: gwt-agent-bridgeが未登録の全エージェント設定
  操作: cleanup_stale_registrations()
  期待: 設定ファイルに変更なし
```

### 1.4 ランタイム検出

```text
テストケース:
- test_detect_runtime_bun_available
  前提: bun がPATHに存在
  期待: "bun" が返る

- test_detect_runtime_node_fallback
  前提: bun がPATHに不存在、node が存在
  期待: "node" が返る

- test_detect_runtime_none
  前提: bun も node も不存在
  期待: エラーが返る
```

## 2. gwt-tauri 結合テスト

### 2.1 WebSocketサーバー

```text
テストファイル: crates/gwt-tauri/src/mcp_ws_server.rs (#[cfg(test)] mod tests)

テストケース:
- test_ws_server_starts_on_random_port
  操作: WebSocketサーバーを起動
  期待: ポート番号が返り、接続可能

- test_ws_server_writes_state_file
  操作: WebSocketサーバーを起動
  期待: ~/.gwt/mcp-state.jsonにポート番号が書き出される

- test_ws_server_accepts_connection
  操作: WebSocketサーバーを起動し、クライアントから接続
  期待: 接続成功

- test_ws_server_routes_jsonrpc
  操作: JSON-RPCリクエストを送信
  期待: 対応するハンドラーが呼ばれ、レスポンスが返る
```

### 2.2 MCPツールハンドラー

```text
テストケース:
- test_handler_list_tabs
  前提: 3タブが稼働中
  操作: gwt_list_tabs リクエスト
  期待: 3タブの情報が返る

- test_handler_get_tab_info_existing
  操作: 既存tab_idでgwt_get_tab_info リクエスト
  期待: タブ詳細が返る

- test_handler_get_tab_info_nonexistent
  操作: 存在しないtab_idでリクエスト
  期待: エラーレスポンス

- test_handler_send_message
  前提: 送信先タブが稼働中
  操作: gwt_send_message リクエスト
  期待: 対象PTYにフォーマット済みメッセージが注入

- test_handler_send_message_sanitize
  操作: 制御文字を含むメッセージで gwt_send_message
  期待: 制御文字がサニタイズされて注入

- test_handler_broadcast
  前提: 3タブ稼働中、タブAから送信
  操作: gwt_broadcast_message リクエスト
  期待: タブB, Cにメッセージ注入、タブAには注入されない

- test_handler_launch_agent_success
  操作: gwt_launch_agent リクエスト
  期待: 新タブが起動し、tab_idが返る

- test_handler_launch_agent_limit
  前提: タブ数が上限に到達
  操作: gwt_launch_agent リクエスト
  期待: エラーレスポンス（上限到達）

- test_handler_launch_agent_rate_limit
  操作: 短時間に連続してgwt_launch_agent
  期待: レート制限超過のエラー

- test_handler_stop_tab
  操作: 稼働中タブに gwt_stop_tab
  期待: タブが停止、成功レスポンス

- test_handler_get_worktree_diff
  前提: ワークツリーに変更あり
  操作: gwt_get_worktree_diff
  期待: git diff結果が返る

- test_handler_get_changed_files
  前提: ワークツリーに変更あり
  操作: gwt_get_changed_files
  期待: 変更ファイル一覧が返る
```

## 3. gwt-mcp-bridge E2Eテスト

### 3.1 MCPプロトコルテスト

```text
テストファイル: gwt-mcp-bridge/tests/e2e.test.ts

テストケース:
- test_mcp_initialize
  操作: MCP initialize ハンドシェイクを送信
  期待: capabilities にtools が含まれる

- test_mcp_list_tools
  操作: tools/list リクエスト
  期待: 8つのツールが定義されている

- test_mcp_call_tool_list_tabs
  前提: モックWebSocketサーバーが稼働
  操作: tools/call gwt_list_tabs
  期待: WebSocketを経由してレスポンスが返る

- test_mcp_ws_disconnect_recovery
  操作: WebSocket接続を切断
  期待: MCPツール呼び出しがエラーを返す
```

## テスト実行順序

1. `cargo test -p gwt-core` (Phase 1 完了後)
2. `cargo test -p gwt-tauri` (Phase 3 完了後)
3. `cd gwt-mcp-bridge && bun test` (Phase 2 完了後)
4. 手動E2Eテスト (Phase 6)
