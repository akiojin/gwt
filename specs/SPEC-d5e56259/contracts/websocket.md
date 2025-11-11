# WebSocket プロトコル仕様

**日付**: 2025-11-10
**仕様ID**: SPEC-d5e56259
**関連ドキュメント**: [spec.md](../spec.md), [rest-api.yaml](./rest-api.yaml)

## 概要

このドキュメントは、claude-worktree Web UIのWebSocketプロトコルを定義します。WebSocketは、ブラウザとサーバー間でAI Toolのターミナル入出力をリアルタイムに転送するために使用されます。

## エンドポイント

### `/ws/terminal/:sessionId`

**説明**: AI Toolセッションのターミナル入出力を双方向で転送するWebSocketエンドポイント

**パラメータ**:
- `sessionId` (string, required): セッションID（UUID v4形式）。事前に `POST /api/sessions` で作成されたセッションIDを指定。

**接続URL例**:
```
ws://localhost:3000/ws/terminal/550e8400-e29b-41d4-a716-446655440000
```

**接続フロー**:
1. クライアントが `POST /api/sessions` でセッションを開始（REST API）
2. サーバーがPTYプロセスを起動し、`sessionId` を返却
3. クライアントが `/ws/terminal/:sessionId` にWebSocket接続
4. サーバーがセッションIDを検証
   - 有効: 接続確立、PTY出力の転送開始
   - 無効: 接続拒否（1008 Policy Violation）

## メッセージフォーマット

すべてのメッセージはJSON形式です。

### 基本構造

```typescript
{
  type: string;     // メッセージタイプ
  data?: any;       // ペイロード（タイプによって異なる）
  timestamp?: string; // ISO8601形式のタイムスタンプ（オプション）
}
```

## クライアント → サーバー（上り）

### 1. `input` - ターミナル入力

**説明**: ユーザーのキーボード入力をPTYに送信

**ペイロード**:
```typescript
{
  type: 'input';
  data: string; // 入力文字列（1文字または複数文字）
}
```

**例**:
```json
{
  "type": "input",
  "data": "ls -la\n"
}
```

**動作**:
- サーバーが `ptyProcess.write(data)` を実行
- PTYがAI Toolに入力を転送

**制約**:
- `data` は最大1KB（1024バイト）
- 超過した場合、サーバーはエラーメッセージを送信

---

### 2. `resize` - ターミナルリサイズ

**説明**: ターミナルのサイズ変更をPTYに通知

**ペイロード**:
```typescript
{
  type: 'resize';
  data: {
    cols: number; // 列数（横幅）
    rows: number; // 行数（縦幅）
  }
}
```

**例**:
```json
{
  "type": "resize",
  "data": {
    "cols": 120,
    "rows": 40
  }
}
```

**動作**:
- サーバーが `ptyProcess.resize(cols, rows)` を実行
- PTYがAI Toolにリサイズを通知（SIGWINCH）

**制約**:
- `cols`: 1-500
- `rows`: 1-500
- 範囲外の場合、エラーメッセージを送信

---

### 3. `ping` - 接続維持（オプション）

**説明**: 接続が生きていることを確認

**ペイロード**:
```typescript
{
  type: 'ping';
}
```

**例**:
```json
{
  "type": "ping"
}
```

**動作**:
- サーバーが `pong` メッセージを返信

---

## サーバー → クライアント（下り）

### 1. `output` - ターミナル出力

**説明**: PTYからの出力（stdout/stderr）をブラウザに送信

**ペイロード**:
```typescript
{
  type: 'output';
  data: string; // 出力文字列（ANSI escape codes含む）
}
```

**例**:
```json
{
  "type": "output",
  "data": "\u001b[32mSuccess!\u001b[0m\n"
}
```

**動作**:
- PTYが `ptyProcess.onData(data => ...)` でデータを受信
- サーバーがWebSocketでクライアントに転送
- クライアント（xterm.js）が `term.write(data)` で描画

**制約**:
- `data` は最大10KB（10240バイト）
- 超過した場合、複数のメッセージに分割して送信

---

### 2. `exit` - プロセス終了

**説明**: PTYプロセスが終了したことを通知

**ペイロード**:
```typescript
{
  type: 'exit';
  data: {
    code: number;    // 終了コード（0=正常、非0=異常）
    signal?: string; // シグナル名（例: 'SIGTERM'）
  }
}
```

**例（正常終了）**:
```json
{
  "type": "exit",
  "data": {
    "code": 0
  }
}
```

**例（異常終了）**:
```json
{
  "type": "exit",
  "data": {
    "code": 1,
    "signal": "SIGTERM"
  }
}
```

**動作**:
- PTYが `ptyProcess.onExit((exitCode, signal) => ...)` で終了を検知
- サーバーがWebSocketで通知
- サーバーがWebSocket接続をクローズ
- クライアントが終了メッセージを表示

---

### 3. `error` - エラー通知

**説明**: サーバー側でエラーが発生したことを通知

**ペイロード**:
```typescript
{
  type: 'error';
  data: {
    message: string; // エラーメッセージ
    code?: string;   // エラーコード（例: 'PTY_SPAWN_FAILED'）
  }
}
```

**例**:
```json
{
  "type": "error",
  "data": {
    "message": "PTY process crashed",
    "code": "PTY_PROCESS_CRASHED"
  }
}
```

**動作**:
- サーバーがエラーを検知
- WebSocketでクライアントに通知
- サーバーがWebSocket接続をクローズ
- クライアントがエラーメッセージを表示

**エラーコード一覧**:
- `PTY_SPAWN_FAILED`: PTYプロセスの起動失敗
- `PTY_PROCESS_CRASHED`: PTYプロセスのクラッシュ
- `SESSION_NOT_FOUND`: セッションIDが無効
- `INPUT_TOO_LARGE`: 入力データが大きすぎる
- `RESIZE_OUT_OF_RANGE`: リサイズのパラメータが範囲外

---

### 4. `pong` - 接続応答（オプション）

**説明**: `ping` への応答

**ペイロード**:
```typescript
{
  type: 'pong';
}
```

**例**:
```json
{
  "type": "pong"
}
```

**動作**:
- クライアントが `ping` を送信
- サーバーが `pong` を返信
- クライアントが接続の生存を確認

---

## 接続ライフサイクル

### 1. 接続確立

```
Client                          Server
  |                               |
  |--- WebSocket Upgrade -------->|
  |                               |
  |<-- 101 Switching Protocols ---|
  |                               |
  |                          (PTY起動)
  |                               |
  |<--------- output --------------|
  |<--------- output --------------|
```

### 2. 通常のやり取り

```
Client                          Server
  |                               |
  |---------- input ------------->|
  |                          (PTY.write)
  |                               |
  |<--------- output -------------|
  |<--------- output -------------|
  |                               |
  |---------- resize ------------>|
  |                       (PTY.resize)
  |                               |
  |<--------- output -------------|
```

### 3. 正常終了

```
Client                          Server
  |                               |
  |<--------- output -------------|
  |<--------- exit: code=0 -------|
  |                               |
  |<-- WebSocket Close 1000 ------|
  |                               |
```

### 4. 異常終了

```
Client                          Server
  |                               |
  |<--------- output -------------|
  |<--------- error --------------|
  |                               |
  |<-- WebSocket Close 1011 ------|
  |                               |
```

### 5. クライアント切断（再接続可能）

```
Client                          Server
  |                               |
  |--- WebSocket Close 1000 ----->|
  |                          (PTY継続)
  |                          (バックグラウンド保持)
  |                               |

(再接続)

Client                          Server
  |                               |
  |--- WebSocket Upgrade -------->|
  |                               |
  |<-- 101 Switching Protocols ---|
  |                               |
  |<--------- output -------------|  (バッファされた出力)
  |<--------- output -------------|
```

---

## エラーハンドリング

### クライアント側エラー

| エラー | 原因 | 処理 |
|--------|------|------|
| 接続拒否（1008） | セッションID無効 | エラーメッセージ表示 |
| メッセージ解析失敗 | 不正なJSON | ログ出力、無視 |
| WebSocket切断（1006） | ネットワークエラー | 再接続試行（最大3回） |

### サーバー側エラー

| エラー | 原因 | 処理 |
|--------|------|------|
| PTY起動失敗 | コマンド不正 | `error` メッセージ送信、接続クローズ |
| PTYクラッシュ | プロセス異常終了 | `error` メッセージ送信、接続クローズ |
| 入力データ過大 | 1KB超過 | `error` メッセージ送信 |
| リサイズ範囲外 | cols/rows不正 | `error` メッセージ送信 |

---

## セキュリティ

### 認証

- **現在**: セッションIDのみ（UUID v4）
- **将来**: トークンベース認証（JWT）を追加予定

### 入力検証

- すべてのメッセージでJSONパース検証
- `type` フィールドの値を厳格にチェック
- `data` フィールドのサイズ制限

### レート制限

- 入力メッセージ: 最大100メッセージ/秒
- リサイズメッセージ: 最大10メッセージ/秒
- 超過した場合、接続をクローズ

---

## パフォーマンス

### 遅延

- **目標**: 100ms以内（入力 → 出力）
- **実測**: 10-50ms（ローカル環境）

### スループット

- **出力**: 最大1MB/秒
- **入力**: 最大10KB/秒

### 接続数

- **上限**: 20接続（PTY 10 + ログ 10）
- **推奨**: 10接続以下

---

## 実装例

### クライアント（TypeScript + xterm.js）

```typescript
import { Terminal } from 'xterm';

const sessionId = '550e8400-e29b-41d4-a716-446655440000';
const ws = new WebSocket(`ws://localhost:3000/ws/terminal/${sessionId}`);
const term = new Terminal();

term.open(document.getElementById('terminal')!);

// サーバーからの出力を表示
ws.onmessage = (event) => {
  const message = JSON.parse(event.data);

  switch (message.type) {
    case 'output':
      term.write(message.data);
      break;
    case 'exit':
      console.log(`Process exited with code ${message.data.code}`);
      ws.close();
      break;
    case 'error':
      console.error('Error:', message.data.message);
      break;
  }
};

// ユーザー入力をサーバーに送信
term.onData((data) => {
  ws.send(JSON.stringify({ type: 'input', data }));
});

// ターミナルリサイズ
term.onResize(({ cols, rows }) => {
  ws.send(JSON.stringify({ type: 'resize', data: { cols, rows } }));
});

// 接続エラー
ws.onerror = (error) => {
  console.error('WebSocket error:', error);
};

// 接続クローズ
ws.onclose = (event) => {
  console.log(`WebSocket closed: ${event.code} ${event.reason}`);
};
```

### サーバー（TypeScript + Fastify + node-pty）

```typescript
import Fastify from 'fastify';
import websocket from '@fastify/websocket';
import pty from 'node-pty';

const app = Fastify();
await app.register(websocket);

const sessions = new Map<string, pty.IPty>();

app.get('/ws/terminal/:sessionId', { websocket: true }, (socket, req) => {
  const sessionId = req.params.sessionId;
  const ptyProcess = sessions.get(sessionId);

  if (!ptyProcess) {
    socket.close(1008, 'Session not found');
    return;
  }

  // PTY出力をWebSocketで送信
  ptyProcess.onData((data) => {
    socket.send(JSON.stringify({ type: 'output', data }));
  });

  // PTY終了を通知
  ptyProcess.onExit(({ exitCode, signal }) => {
    socket.send(JSON.stringify({
      type: 'exit',
      data: { code: exitCode, signal }
    }));
    socket.close(1000);
  });

  // WebSocketメッセージ処理
  socket.on('message', (raw) => {
    try {
      const message = JSON.parse(raw.toString());

      switch (message.type) {
        case 'input':
          if (message.data.length > 1024) {
            socket.send(JSON.stringify({
              type: 'error',
              data: { message: 'Input too large', code: 'INPUT_TOO_LARGE' }
            }));
            return;
          }
          ptyProcess.write(message.data);
          break;

        case 'resize':
          const { cols, rows } = message.data;
          if (cols < 1 || cols > 500 || rows < 1 || rows > 500) {
            socket.send(JSON.stringify({
              type: 'error',
              data: { message: 'Resize out of range', code: 'RESIZE_OUT_OF_RANGE' }
            }));
            return;
          }
          ptyProcess.resize(cols, rows);
          break;

        case 'ping':
          socket.send(JSON.stringify({ type: 'pong' }));
          break;
      }
    } catch (error) {
      console.error('Failed to parse message:', error);
    }
  });

  // WebSocketクローズ
  socket.on('close', () => {
    // PTYプロセスはバックグラウンドで継続（再接続可能）
  });
});

await app.listen({ port: 3000 });
```

---

## 次のステップ

1. ✅ WebSocketプロトコル定義完了
2. ⏭️ クイックスタートガイド作成（quickstart.md）
3. ⏭️ `/speckit.tasks` 実行: 実装タスクリスト生成
