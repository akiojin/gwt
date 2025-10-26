# データモデル

**仕様ID**: `SPEC-6d501fd0` | **日付**: 2025-01-26
**目的**: 仮想ターミナル機能の主要エンティティとデータ構造を定義

## 1. エンティティ概要

この機能では、以下の主要エンティティを使用します：

| エンティティ | 目的 | ライフサイクル |
|------------|------|--------------|
| TerminalSession | AIツール実行セッション情報 | 起動時に作成、終了時に破棄 |
| TerminalOutput | ターミナル出力データ | セッション中にリアルタイムで蓄積 |
| PtyProcess | PTYプロセスの状態 | PTY起動時に作成、終了時に破棄 |
| LogFile | 保存されたログファイル情報 | ログ保存時に作成 |

## 2. エンティティ詳細

### 2.1 TerminalSession

**説明**: AIツールの実行セッション情報を保持します。

**TypeScript型定義**:
```typescript
interface TerminalSession {
  // セッション識別子（UUID）
  id: string;

  // 選択されたブランチ名
  branch: string;

  // 選択されたAIツール
  tool: 'claude-code' | 'codex-cli';

  // 実行モード
  mode: 'normal' | 'continue' | 'resume';

  // 作業ディレクトリパス（worktreeパス）
  worktreePath: string;

  // セッション開始時刻
  startTime: Date;

  // セッション終了時刻（終了時にセット）
  endTime?: Date;

  // スキップパーミッションフラグ
  skipPermissions: boolean;
}
```

**属性詳細**:

| 属性 | 型 | 必須 | 説明 | 検証ルール |
|------|-----|-----|------|----------|
| id | string | ✓ | セッション識別子 | UUID v4 形式 |
| branch | string | ✓ | ブランチ名 | 1文字以上 |
| tool | AITool | ✓ | AIツール種別 | 'claude-code' または 'codex-cli' |
| mode | ExecutionMode | ✓ | 実行モード | 'normal', 'continue', 'resume' のいずれか |
| worktreePath | string | ✓ | 作業ディレクトリ | 絶対パス |
| startTime | Date | ✓ | 開始時刻 | ISO 8601 形式 |
| endTime | Date | - | 終了時刻 | ISO 8601 形式、startTime より後 |
| skipPermissions | boolean | ✓ | パーミッションスキップ | true or false |

**状態遷移**:
```
[作成] → [実行中] → [終了]
```

### 2.2 TerminalOutput

**説明**: ターミナルに表示される出力データを表します。

**TypeScript型定義**:
```typescript
interface TerminalOutput {
  // 出力行の識別子
  id: string;

  // 所属するセッションID
  sessionId: string;

  // 出力されたタイムスタンプ
  timestamp: Date;

  // 出力内容（ANSI制御コード含む）
  content: string;

  // 標準エラー出力かどうか
  isError: boolean;
}
```

**属性詳細**:

| 属性 | 型 | 必須 | 説明 | 検証ルール |
|------|-----|-----|------|----------|
| id | string | ✓ | 出力行識別子 | UUID v4 形式 |
| sessionId | string | ✓ | セッションID | 有効なTerminalSession.id |
| timestamp | Date | ✓ | 出力時刻 | ISO 8601 形式 |
| content | string | ✓ | 出力内容 | ANSI制御コード含む |
| isError | boolean | ✓ | エラー出力フラグ | true or false |

**関係**:
- TerminalSession (1) --- (*) TerminalOutput

### 2.3 PtyProcess

**説明**: PTYプロセスの状態を管理します。

**TypeScript型定義**:
```typescript
interface PtyProcess {
  // プロセスID
  pid: number;

  // PTYインスタンス（node-pty）
  ptyInstance: IPty;

  // プロセスステータス
  status: 'running' | 'paused' | 'stopped';

  // 終了コード（終了時にセット）
  exitCode?: number;

  // エラーメッセージ（異常終了時にセット）
  errorMessage?: string;
}
```

**属性詳細**:

| 属性 | 型 | 必須 | 説明 | 検証ルール |
|------|-----|-----|------|----------|
| pid | number | ✓ | プロセスID | 正の整数 |
| ptyInstance | IPty | ✓ | PTYインスタンス | node-ptyのIPtyインターフェース |
| status | ProcessStatus | ✓ | ステータス | 'running', 'paused', 'stopped' のいずれか |
| exitCode | number | - | 終了コード | 整数（0-255） |
| errorMessage | string | - | エラーメッセージ | 任意の文字列 |

**状態遷移**:
```
[running] ←→ [paused] (Ctrl+Z)
[running] → [stopped] (Ctrl+C / 正常終了 / 異常終了)
[paused] → [stopped] (Ctrl+C)
```

**制約**:
- Unix系プラットフォームのみ `paused` ステータスをサポート
- Windows では `running` → `stopped` のみ

### 2.4 LogFile

**説明**: 保存されたログファイルの情報を保持します。

**TypeScript型定義**:
```typescript
interface LogFile {
  // ログファイルの絶対パス
  filePath: string;

  // ログ保存時刻
  savedAt: Date;

  // ファイルサイズ（バイト）
  size: number;

  // 所属するセッションID
  sessionId: string;
}
```

**属性詳細**:

| 属性 | 型 | 必須 | 説明 | 検証ルール |
|------|-----|-----|------|----------|
| filePath | string | ✓ | ログファイルパス | 絶対パス、`.logs/` ディレクトリ内 |
| savedAt | Date | ✓ | 保存時刻 | ISO 8601 形式 |
| size | number | ✓ | ファイルサイズ | 正の整数（バイト単位） |
| sessionId | string | ✓ | セッションID | 有効なTerminalSession.id |

**ファイル命名規則**:
```
.logs/{YYYY-MM-DDTHH-mm-ss}-{tool}-{branch}.log
```

例:
```
.logs/2025-01-26T10-30-45-claude-code-feature-terminal.log
```

**関係**:
- TerminalSession (1) --- (*) LogFile

## 3. エンティティ関係図

```
TerminalSession (1)
  ├── (1) PtyProcess
  ├── (*) TerminalOutput
  └── (*) LogFile
```

**説明**:
- 1つのTerminalSessionは1つのPtyProcessを持つ
- 1つのTerminalSessionは複数のTerminalOutputを持つ（ストリーミング）
- 1つのTerminalSessionは複数のLogFileを持つ可能性がある（複数回保存）

## 4. データフロー

### 4.1 セッション開始フロー

```
1. ユーザーがブランチ・ツール・モードを選択
   ↓
2. TerminalSession作成（id生成、startTime記録）
   ↓
3. PtyProcess作成（PTY起動、status = 'running'）
   ↓
4. TerminalScreenにレンダリング
```

### 4.2 出力データフロー

```
PTY出力イベント
  ↓
TerminalOutput作成（content, timestamp記録）
  ↓
outputBuffer配列に追加
  ↓
React stateに反映（バッチ処理: 16ms毎）
  ↓
TerminalScreenに表示
```

### 4.3 ログ保存フロー

```
Ctrl+S押下
  ↓
outputBufferから全出力を取得
  ↓
ファイルパス生成（.logs/{timestamp}-{tool}-{branch}.log）
  ↓
ファイルに書き込み（fs.promises.writeFile）
  ↓
LogFile作成（filePath, savedAt, size記録）
  ↓
保存完了メッセージ表示
```

### 4.4 セッション終了フロー

```
AIツール終了（exitイベント）
  ↓
TerminalSession更新（endTime記録）
  ↓
PtyProcess更新（status = 'stopped', exitCode記録）
  ↓
TerminalScreen終了
  ↓
BranchListScreenに遷移
```

## 5. ストレージ

### 5.1 メモリ内データ

**React State**:
- `terminalSession: TerminalSession | null`
- `outputBuffer: TerminalOutput[]`
- `ptyProcess: PtyProcess | null`

**ライフサイクル**:
- TerminalScreenマウント時に初期化
- TerminalScreenアンマウント時にクリア

### 5.2 ファイルシステム

**ログファイル**:
- パス: `.logs/` ディレクトリ
- フォーマット: プレーンテキスト（ANSI制御コード含む）
- 保持期間: 無期限（手動削除が必要）

## 6. 検証ルール

### 6.1 TerminalSession検証

```typescript
function validateTerminalSession(session: TerminalSession): boolean {
  // ID
  if (!session.id || !/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(session.id)) {
    return false;
  }

  // branch
  if (!session.branch || session.branch.length === 0) {
    return false;
  }

  // tool
  if (!['claude-code', 'codex-cli'].includes(session.tool)) {
    return false;
  }

  // mode
  if (!['normal', 'continue', 'resume'].includes(session.mode)) {
    return false;
  }

  // worktreePath
  if (!session.worktreePath || !path.isAbsolute(session.worktreePath)) {
    return false;
  }

  // startTime
  if (!(session.startTime instanceof Date) || isNaN(session.startTime.getTime())) {
    return false;
  }

  // endTime (optional)
  if (session.endTime) {
    if (!(session.endTime instanceof Date) || isNaN(session.endTime.getTime())) {
      return false;
    }
    if (session.endTime <= session.startTime) {
      return false;
    }
  }

  return true;
}
```

### 6.2 LogFile検証

```typescript
function validateLogFilePath(filePath: string): boolean {
  // 絶対パスであること
  if (!path.isAbsolute(filePath)) {
    return false;
  }

  // .logs/ ディレクトリ内であること
  const dirName = path.dirname(filePath);
  if (!dirName.endsWith('.logs')) {
    return false;
  }

  // ファイル名がパターンに一致すること
  const fileName = path.basename(filePath);
  const pattern = /^\d{4}-\d{2}-\d{2}T\d{2}-\d{2}-\d{2}-(claude-code|codex-cli)-.+\.log$/;
  return pattern.test(fileName);
}
```

## 7. パフォーマンス考慮事項

### 7.1 出力バッファリング

**問題**: 高頻度の出力でReactレンダリングが追いつかない

**解決策**: バッチ処理
```typescript
// 16ms（60fps）毎にReact stateを更新
const UPDATE_INTERVAL = 16;

let pendingOutputs: TerminalOutput[] = [];

ptyProcess.onData((data: string) => {
  const output: TerminalOutput = {
    id: uuid(),
    sessionId: session.id,
    timestamp: new Date(),
    content: data,
    isError: false,
  };
  pendingOutputs.push(output);
});

setInterval(() => {
  if (pendingOutputs.length > 0) {
    setOutputBuffer(prev => [...prev, ...pendingOutputs]);
    pendingOutputs = [];
  }
}, UPDATE_INTERVAL);
```

### 7.2 出力行数制限

**問題**: 大量の出力でメモリ消費が増大

**解決策**: スクロールバッファ（最新N行のみ保持）
```typescript
const MAX_OUTPUT_LINES = 10000;

function addOutput(output: TerminalOutput) {
  setOutputBuffer(prev => {
    const newBuffer = [...prev, output];
    if (newBuffer.length > MAX_OUTPUT_LINES) {
      return newBuffer.slice(-MAX_OUTPUT_LINES);
    }
    return newBuffer;
  });
}
```

## 8. 次のステップ

✅ **データモデル定義完了**

⏭️ **quickstart.md**: 開発者向けガイドの作成

⏭️ **/speckit.tasks**: 実装タスクの生成
