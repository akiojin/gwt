# タスクリスト: エージェントタブ MCP通信ブリッジ

## Phase 1: 基盤インフラ（WebSocketサーバー + 接続情報管理）

- [x] T1: gwt-tauriにtokio-tungstenite依存を追加し、WebSocketサーバーモジュールを作成
- [x] T2: アプリ起動時にランダムポートでWebSocketサーバーを起動するライフサイクル管理を実装
- [x] T3: `~/.gwt/mcp-state.json` への接続情報書き出し・起動時クリーンアップ・終了時削除を実装
- [x] T4: JSON-RPCメッセージのルーティング基盤（リクエスト→ハンドラー→レスポンス）を実装

## Phase 2: MCPブリッジプロセス実装

- [x] T5: `gwt-mcp-bridge/` プロジェクト初期化（package.json, tsconfig.json, esbuild設定）
- [x] T6: @modelcontextprotocol/sdk でstdio MCPサーバーの骨格を実装
- [x] T7: ws ライブラリで `~/.gwt/mcp-state.json` を読んでWebSocket接続するクライアントを実装
- [x] T8: 8つのMCPツール定義（スキーマ + description）を登録
- [x] T9: MCPリクエスト → WebSocket転送 → レスポンス返却のパイプラインを実装
- [x] T10: esbuildで単一JSファイルにバンドルし、tauri.conf.jsonのresourcesに追加

## Phase 3: MCPツール実装（gwt-tauri側ハンドラー）

- [x] T11: `gwt_list_tabs` / `gwt_get_tab_info` ハンドラーを実装（TerminalManager状態参照）
- [x] T12: `gwt_send_message` ハンドラーを実装（PTYフォーマット済み注入、制御文字サニタイズ）
- [x] T13: `gwt_broadcast_message` ハンドラーを実装（送信元除外の全タブ配送）
- [x] T14: `gwt_launch_agent` ハンドラーを実装（タブ数上限 + レート制限付き）
- [x] T15: `gwt_stop_tab` ハンドラーを実装
- [x] T16: `gwt_get_worktree_diff` / `gwt_get_changed_files` ハンドラーを実装（gwt-core git機能経由）

## Phase 4: エージェント設定の動的登録・解除

- [x] T17: `gwt-core/src/config/mcp_registration.rs` を作成し、設定ファイル読み書きの抽象化レイヤーを実装
- [x] T18: Claude Code設定（`~/.claude.json`, JSON）の読み書き・登録・削除を実装
- [x] T19: Codex設定（`~/.codex/config.toml`, TOML）の読み書き・登録・削除を実装
- [x] T20: Gemini設定（`~/.gemini/settings.json`, JSON）の読み書き・登録・削除を実装
- [x] T21: gwt起動時の自動登録（クリーンアップ→ランタイム検出→登録）を実装
- [x] T22: gwt終了時の自動解除を実装
- [x] T23: ブリッジ側のWebSocket接続失敗時の自己クリーンアップを実装

## Phase 5: can_use_mcp修正 + ランタイム自動検出

- [x] T24: Codex, GeminiのAgentCapabilitiesで `can_use_mcp` を `true` に変更
- [x] T25: ランタイム自動検出（bun優先→nodeフォールバック）を実装し、設定登録のcommandフィールドに反映

## Phase 6: テスト + 検証

- [x] T26: mcp_registration.rs のユニットテスト（JSON/TOML読み書き、登録・削除・クリーンアップ）
- [x] T27: WebSocketサーバーの結合テスト（接続・切断・メッセージルーティング）
- [ ] T28: MCPブリッジのE2Eテスト（ツール呼び出し → WebSocket → Tauri → レスポンス）
- [x] T29: cargo clippy, cargo fmt, svelte-check による全体品質チェック
- [ ] T30: 手動検証（gwt起動→MCP登録確認→ツール呼び出し→終了→登録解除確認）
