# 実装計画: エージェントタブ MCP通信ブリッジ

## 目的

- 各エージェント（Claude Code, Codex, Gemini）がMCPツール経由でgwtと双方向通信できる基盤を構築
- gwt-tauri内にWebSocketサーバーを追加し、MCPブリッジプロセスからの接続を処理
- Node.js/Bunで薄いMCPブリッジを実装し、Tauri resourceとしてバンドル
- 全エージェントのグローバル設定にMCPサーバーを動的登録・解除

## 実装方針

### Phase 1: 基盤インフラ（WebSocketサーバー + 接続情報管理）

1. gwt-tauriにtokio-tungsteniteベースのWebSocketサーバーを追加
2. 起動時にランダムポートでlistenし、`~/.gwt/mcp-state.json`にポート情報を書き出す
3. WebSocketメッセージのJSON-RPCルーティング基盤を実装
4. gwt終了時に`mcp-state.json`を削除

### Phase 2: MCPブリッジプロセス実装

1. `gwt-mcp-bridge/` ディレクトリを作成し、TypeScriptプロジェクトを初期化
2. `@modelcontextprotocol/sdk` でstdio MCPサーバーを実装
3. `ws` ライブラリでTauriのWebSocketサーバーに接続
4. 8つのMCPツールのスタブを定義
5. MCPリクエスト→WebSocket→Tauri→WebSocket→MCPレスポンスのパイプラインを実装
6. esbuildで単一JSファイルにバンドル
7. `tauri.conf.json` のresourcesにバンドル済みJSを追加

### Phase 3: MCPツール実装（gwt-tauri側ハンドラー）

1. **gwt_list_tabs / gwt_get_tab_info**: 既存のTerminalManager状態からタブ情報を収集
2. **gwt_send_message / gwt_broadcast_message**: PTYへのフォーマット済みメッセージ注入
   - メッセージフォーマット: `[gwt msg from <sender>]: <message>`
   - 構造化テキストのみに制限（改行・制御文字のサニタイズ）
3. **gwt_launch_agent / gwt_stop_tab**: 既存のlaunch_agent_for_project_root/停止処理をラップ
   - タブ数上限チェック（デフォルト8）
   - レート制限（1分あたりN回）
4. **gwt_get_worktree_diff / gwt_get_changed_files**: gwt-coreのgit機能を呼び出し

### Phase 4: エージェント設定の動的登録・解除

1. `gwt-core/src/config/` に `mcp_registration.rs` を追加
2. 各エージェント設定ファイルの読み書きモジュール:
   - Claude Code: `~/.claude.json` (JSON, `mcpServers` キー)
   - Codex: `~/.codex/config.toml` (TOML, `[mcp_servers]` セクション)
   - Gemini: `~/.gemini/settings.json` (JSON, `mcpServers` キー)
3. gwt起動時: 既存登録をクリーンアップ → ランタイム検出 → ブリッジパス解決 → 登録
4. gwt終了時: 全エージェント設定から `gwt-agent-bridge` を削除
5. ブリッジ側: WebSocket接続失敗時に設定から自身を削除する自己クリーンアップ

### Phase 5: can_use_mcp修正 + ランタイム自動検出

1. `gwt-core/src/agent/` のCodex/Gemini実装で `can_use_mcp` を `true` に変更
2. ブリッジ起動コマンドのランタイム検出:
   - `which bun` → 成功ならbun
   - フォールバック: node
3. `.mcp.json` の `command` フィールドに検出されたランタイムパスを設定

### Phase 6: テスト + 検証

1. gwt-core: mcp_registration のユニットテスト
   - JSON/TOML設定の読み書き
   - 登録・削除・クリーンアップ
2. gwt-tauri: WebSocketサーバーの結合テスト
   - 接続・切断・メッセージルーティング
3. ブリッジ: MCPツール呼び出しのE2Eテスト
4. 手動検証: Claude Code/Codex/Geminiからの実際のMCPツール呼び出し

## テスト計画

### unit test (gwt-core / mcp_registration.rs)

- JSON設定ファイルの読み込み・書き込み・MCP登録追加・削除
- TOML設定ファイルの読み込み・書き込み・MCP登録追加・削除
- 残存登録のクリーンアップ検出
- ランタイム検出ロジック

### unit test (gwt-tauri / WebSocket)

- WebSocketサーバー起動・ポートバインド
- JSON-RPCメッセージのルーティング
- 各MCPツールハンドラーの動作

### 手動確認

- gwtを起動し、各エージェント設定にgwt-agent-bridgeが登録されることを確認
- Claude CodeからMCPツール（gwt_list_tabs等）が使えることを確認
- gwt終了後に設定から登録が削除されることを確認
- gwt強制終了後の再起動でクリーンアップが動作することを確認
