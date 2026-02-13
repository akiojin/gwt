# 機能仕様: エージェントタブ MCP通信ブリッジ

**仕様ID**: `SPEC-a7b3c9e1`
**作成日**: 2026-02-13
**ステータス**: 確定
**カテゴリ**: Core / Agent / MCP

**入力**: ユーザー説明: "エージェントタブに対する通信APIとして、MCPサーバーを用意し、各エージェントのグローバル設定に追加したい"

## 背景

gwt では複数のエージェントタブ（Claude Code, Codex, Gemini 等）を並行して実行できる。
現在、タブ間の通信はmaster agentがOpenAI Responses APIのtool_calls経由で
PTYツール（`send_keys_to_pane`, `capture_scrollback_tail`）を使って行う一方向の制御のみ。

各サブエージェント（Claude Code等）側から能動的にgwtの状態を問い合わせたり、
他のタブにメッセージを送ったりする手段がない。

本仕様では、MCP (Model Context Protocol) サーバーを gwt に組み込み、
各エージェントが MCP ツール経由で gwt と双方向通信できる基盤を構築する。

## 設計判断

### アーキテクチャ: ブリッジプロセス方式

MCPのstdioトランスポートでは、エージェントが子プロセスとしてMCPサーバーを起動する。
gwt-tauriプロセス内で直接stdioを処理することはできないため、
薄いブリッジプロセスを介してTauriと接続する。

```text
Agent (Claude Code等)
  ↕ stdio (MCP protocol)
MCP Bridge Process (Node.js/Bun, 単一JSファイル)
  ↕ WebSocket
gwt-tauri (Rust, WebSocket Server)
```

### 通信方向: 双方向

- **エージェント → gwt**: エージェントがMCPツールを呼び出し、gwtの状態取得やメッセージ送信
- **gwt → エージェント**: メッセージ受信時にPTYへフォーマット済みテキストを注入

### 既存ツールとの共存

master agentのPTYツール（`send_keys_to_pane`, `capture_scrollback_tail`等）は
別レイヤーとしてそのまま維持する。MCPはサブエージェント側の通信手段として追加。

### セキュリティ: 構造化メッセージのみ

MCPツール経由のタブ間通信では**構造化メッセージのみ**を許可する。
任意のPTYコマンド実行（send_keys相当）はブロックし、
フォーマット済みメッセージテキストのみを対象タブに注入する。

### メッセージ配送: 即時PTY注入

受信側エージェントのPTYに `[gwt msg from <送信元>]: <メッセージ>` 形式で即時注入する。
キューイングは行わず、タイミング制御は送信側エージェントの責任とする。

### 自己増殖制限

エージェントがMCP経由で別のエージェントタブを起動できるため、
タブ数上限とレート制限の両方を実装する。

## ユーザーシナリオとテスト

### ユーザーストーリー 1: タブ一覧取得（優先度: 高）

エージェントが他のタブの存在を認識し、通信先を特定できる。

#### 受け入れシナリオ

**シナリオ 1.1**: タブ一覧の取得

- **前提条件**: gwt で 3 つのエージェントタブが稼働中（Claude Code x2, Codex x1）
- **操作**: エージェントAが `gwt_list_tabs()` を呼び出す
- **期待結果**: 3タブの情報（tab_id, agent_type, branch, status）がJSON配列で返る

**シナリオ 1.2**: タブ詳細の取得

- **前提条件**: 特定のtab_idが既知
- **操作**: `gwt_get_tab_info(tab_id)` を呼び出す
- **期待結果**: タブの詳細情報（agent_type, branch, worktree_path, session_id, status）が返る

### ユーザーストーリー 2: タブ間メッセージング（優先度: 高）

エージェントが他のタブに構造化メッセージを送信できる。

#### 受け入れシナリオ

**シナリオ 2.1**: 特定タブへのメッセージ送信

- **前提条件**: エージェントAとエージェントBが稼働中
- **操作**: エージェントAが `gwt_send_message(B_tab_id, "fix completed on feature/auth")` を呼び出す
- **期待結果**: エージェントBのPTYに `[gwt msg from <A_tab_id>]: fix completed on feature/auth` が注入される

**シナリオ 2.2**: ブロードキャストメッセージ

- **前提条件**: 3つのエージェントタブが稼働中
- **操作**: エージェントAが `gwt_broadcast_message("rebasing main, please wait")` を呼び出す
- **期待結果**: エージェントA以外の全タブのPTYにメッセージが注入される

**シナリオ 2.3**: 存在しないタブへの送信

- **操作**: `gwt_send_message("nonexistent-id", "hello")` を呼び出す
- **期待結果**: エラーレスポンス（タブが見つからない旨）が返る

### ユーザーストーリー 3: タブ制御（優先度: 中）

エージェントが他のエージェントタブを起動・停止できる。

#### 受け入れシナリオ

**シナリオ 3.1**: 新規タブ起動

- **前提条件**: 現在2タブ稼働中、上限未到達
- **操作**: `gwt_launch_agent("claude", "feature/auth")` を呼び出す
- **期待結果**: 新しいタブが起動し、新タブのtab_idが返る

**シナリオ 3.2**: タブ数上限での起動拒否

- **前提条件**: タブ数が上限に到達
- **操作**: `gwt_launch_agent(...)` を呼び出す
- **期待結果**: エラーレスポンス（上限到達の旨）が返る

**シナリオ 3.3**: レート制限

- **前提条件**: 短時間に連続してタブを起動
- **操作**: 1分以内に5回 `gwt_launch_agent(...)` を呼び出す
- **期待結果**: レート制限超過のエラーが返る

**シナリオ 3.4**: タブ停止

- **操作**: `gwt_stop_tab(tab_id)` を呼び出す
- **期待結果**: 指定タブが停止し、成功レスポンスが返る

### ユーザーストーリー 4: ファイル・差分共有（優先度: 中）

エージェントが他のタブのワークツリーの変更状態を確認できる。

#### 受け入れシナリオ

**シナリオ 4.1**: 変更ファイル一覧取得

- **前提条件**: 対象タブのワークツリーに変更ファイルがある
- **操作**: `gwt_get_changed_files(tab_id)` を呼び出す
- **期待結果**: 変更ファイルの一覧（パス、ステータス: added/modified/deleted）が返る

**シナリオ 4.2**: ワークツリー差分取得

- **操作**: `gwt_get_worktree_diff(tab_id)` を呼び出す
- **期待結果**: `git diff` 相当の差分テキストが返る

### ユーザーストーリー 5: MCPサーバー自動登録（優先度: 高）

gwt起動時にMCPサーバーが各エージェントの設定に自動登録される。

#### 受け入れシナリオ

**シナリオ 5.1**: gwt起動時の登録

- **前提条件**: gwt未起動、各エージェント設定にgwt-agent-bridgeが未登録
- **操作**: gwtを起動する
- **期待結果**: 以下の設定ファイルにMCPサーバーが登録される
  - `~/.claude.json` (JSON, `mcpServers`)
  - `~/.codex/config.toml` (TOML, `[mcp_servers.gwt-agent-bridge]`)
  - `~/.gemini/settings.json` (JSON, `mcpServers`)

**シナリオ 5.2**: gwt終了時の解除

- **操作**: gwtを正常終了する
- **期待結果**: 上記3ファイルからgwt-agent-bridgeの登録が削除される

**シナリオ 5.3**: クラッシュ後の起動時クリーンアップ

- **前提条件**: 前回gwtがクラッシュし、設定にMCPサーバーが残存
- **操作**: gwtを再起動する
- **期待結果**: 残存登録が検出・削除された後、再登録される

**シナリオ 5.4**: ブリッジヘルスチェック

- **前提条件**: ブリッジプロセスがTauriとのWebSocket接続に失敗
- **操作**: ブリッジプロセスが接続失敗を検出
- **期待結果**: ブリッジが該当エージェントの設定からMCPサーバー登録を削除する

### エッジケース

- **gwt外でのエージェント使用**: gwt未起動時にClaude Codeを単独起動した場合、
  ブリッジプロセスが起動できず/接続できず、サイレントに失敗する
- **複数gwtインスタンス**: 同一マシンで複数gwtを起動した場合、
  WebSocketポートが競合する可能性がある
- **Docker環境**: Dockerコンテナ内のエージェントからはlocalhost WebSocket接続できない

## 要件

### 機能要件

- **FR-001**: gwt-tauriはWebSocketサーバーを起動し、MCPブリッジからの接続を受け付けなければ**ならない**
- **FR-002**: MCPブリッジプロセスはstdioでMCPプロトコルを処理し、WebSocketでgwt-tauriと通信しなければ**ならない**
- **FR-003**: MCPブリッジはesbuildで単一JSファイルにバンドルされ、Tauri resourceとして配布されなければ**ならない**
- **FR-004**: ブリッジプロセスのランタイムはbunを優先し、なければnodeにフォールバックしなければ**ならない**
- **FR-005**: `~/.gwt/mcp-state.json` にWebSocketポート等の接続情報を書き出さなければ**ならない**
- **FR-006**: ブリッジプロセスは起動時に `~/.gwt/mcp-state.json` を読んでWebSocket接続しなければ**ならない**
- **FR-007**: `gwt_list_tabs()` はアクティブな全タブの情報を返さなければ**ならない**
- **FR-008**: `gwt_get_tab_info(tab_id)` は指定タブの詳細情報を返さなければ**ならない**
- **FR-009**: `gwt_send_message(target_tab_id, message)` は対象タブのPTYにフォーマット済みメッセージを注入しなければ**ならない**
- **FR-010**: `gwt_broadcast_message(message)` は送信元以外の全タブにメッセージを注入しなければ**ならない**
- **FR-011**: `gwt_launch_agent(agent_id, branch, ...)` は新規エージェントタブを起動しなければ**ならない**
- **FR-012**: `gwt_stop_tab(tab_id)` は指定タブを停止しなければ**ならない**
- **FR-013**: `gwt_get_worktree_diff(tab_id)` はTauri経由でgit diff結果を返さなければ**ならない**
- **FR-014**: `gwt_get_changed_files(tab_id)` はTauri経由で変更ファイル一覧を返さなければ**ならない**
- **FR-015**: メッセージ送信は構造化テキストのみに制限し、任意のPTYコマンド実行を**ブロック**しなければ**ならない**
- **FR-016**: タブ起動にタブ数上限とレート制限の両方を実装しなければ**ならない**
- **FR-017**: gwt起動時にClaude Code (`~/.claude.json`)、Codex (`~/.codex/config.toml`)、Gemini (`~/.gemini/settings.json`) のグローバル設定にMCPサーバーを動的に登録しなければ**ならない**
- **FR-018**: gwt終了時に上記3つの設定ファイルからMCPサーバー登録を削除しなければ**ならない**
- **FR-019**: gwt起動時に前回クラッシュによる残存登録を検出・クリーンアップしなければ**ならない**
- **FR-020**: ブリッジプロセスがWebSocket接続失敗時に設定ファイルからMCPサーバー登録を削除しなければ**ならない**
- **FR-021**: Codex, GeminiのAgentCapabilitiesで `can_use_mcp` を `true` に修正しなければ**ならない**

### 非機能要件

- **NFR-001**: ブリッジプロセスの起動からMCPツール利用可能までの時間は3秒以内でなければ**ならない**
- **NFR-002**: WebSocket接続のレイテンシはMCPツール応答時間に100ms以上の追加遅延を加えては**ならない**
- **NFR-003**: ブリッジの単一JSファイルサイズは5MB以下でなければ**ならない**

## インターフェース

### MCPツール定義

| ツール名 | パラメータ | 戻り値 |
|---|---|---|
| `gwt_list_tabs` | なし | `Tab[]` |
| `gwt_get_tab_info` | `tab_id: string` | `TabDetail` |
| `gwt_send_message` | `target_tab_id: string, message: string` | `{ success: boolean }` |
| `gwt_broadcast_message` | `message: string` | `{ sent_count: number }` |
| `gwt_launch_agent` | `agent_id: string, branch: string` | `{ tab_id: string }` |
| `gwt_stop_tab` | `tab_id: string` | `{ success: boolean }` |
| `gwt_get_worktree_diff` | `tab_id: string` | `{ diff: string }` |
| `gwt_get_changed_files` | `tab_id: string` | `ChangedFile[]` |

### WebSocketメッセージプロトコル (Bridge ↔ Tauri)

JSON-RPC形式でリクエスト/レスポンスを交換する。

### MCPサーバー登録名

- サーバー名: `gwt-agent-bridge`

### 設定ファイルフォーマット

**Claude Code** (`~/.claude.json`):

```json
{
  "mcpServers": {
    "gwt-agent-bridge": {
      "command": "<runtime>",
      "args": ["<bridge.js path>"],
      "env": {}
    }
  }
}
```

**Codex** (`~/.codex/config.toml`):

```toml
[mcp_servers.gwt-agent-bridge]
command = "<runtime>"
args = ["<bridge.js path>"]
```

**Gemini** (`~/.gemini/settings.json`):

```json
{
  "mcpServers": {
    "gwt-agent-bridge": {
      "command": "<runtime>",
      "args": ["<bridge.js path>"]
    }
  }
}
```

## 範囲外

- MCPブリッジ経由でのPTY任意コマンド実行
- Docker環境内エージェントからのWebSocket接続
- 複数gwtインスタンスのポート競合解決
- MCPリソースやプロンプトの提供（ツールのみ）
- MCP以外のプロトコル（gRPC等）対応
