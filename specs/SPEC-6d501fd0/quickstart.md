# クイックスタートガイド

**仕様ID**: `SPEC-6d501fd0` | **日付**: 2025-01-26
**対象**: 開発者（実装・テスト・デバッグ担当者）

## 1. セットアップ

### 1.1 前提条件

- Bun 1.0+ がインストールされていること
- Git リポジトリ内で作業していること
- macOS, Linux, または Windows（WSL推奨）

### 1.2 依存関係のインストール

```bash
# プロジェクトルートで実行
cd /path/to/claude-worktree

# 依存関係をインストール
bun install

# node-pty を追加（疑似端末ライブラリ）
bun add node-pty
bun add -d @types/node-pty
```

**注意**: node-ptyはネイティブモジュールのため、初回インストール時にコンパイルが実行されます。

### 1.3 ビルド

```bash
# TypeScriptをコンパイル
bun run build

# ビルド結果を確認
ls dist/
```

### 1.4 動作確認

```bash
# アプリケーションを起動
bun run start

# または
bunx .
```

## 2. 開発ワークフロー

### 2.1 TDDアプローチ

このプロジェクトはTest-Driven Development（TDD）を採用しています。

**Red-Green-Refactorサイクル**:
```
1. テストを書く（Red）
   ↓
2. テストが失敗することを確認
   ↓
3. 最小限の実装でテストをパス（Green）
   ↓
4. リファクタリング（Refactor）
   ↓
（1に戻る）
```

**例**: PtyManagerのテスト駆動開発
```typescript
// 1. テストを書く
describe('PtyManager', () => {
  test('should spawn PTY process', () => {
    const manager = new PtyManager();
    const pty = manager.spawn('echo', ['hello']);
    expect(pty.pid).toBeGreaterThan(0);
  });
});

// 2. 実行して失敗を確認
// bun test
// → FAIL: PtyManager is not defined

// 3. 最小限の実装
export class PtyManager {
  spawn(command: string, args: string[]) {
    return pty.spawn(command, args, { /* ... */ });
  }
}

// 4. テストをパス
// bun test
// → PASS

// 5. リファクタリング（必要に応じて）
```

### 2.2 テスト実行

```bash
# 全テストを実行
bun test

# 監視モード（ファイル変更時に自動実行）
bun run test:watch

# 特定のファイルのみテスト
bun test src/pty/PtyManager.test.ts

# カバレッジレポート
bun run test:coverage
```

### 2.3 開発サーバー

```bash
# TypeScriptを監視モードでコンパイル
bun run dev

# 別のターミナルで実行
bun run start
```

## 3. TerminalScreenの統合

### 3.1 基本的な使い方

**ファイル**: `src/ui/components/screens/TerminalScreen.tsx`

```typescript
import React, { useEffect } from 'react';
import { Box, Text, useInput } from 'ink';
import { usePtyProcess } from '../../hooks/usePtyProcess.js';

export interface TerminalScreenProps {
  session: TerminalSession;
  onBack: () => void;
}

export function TerminalScreen({ session, onBack }: TerminalScreenProps) {
  const { ptyProcess, outputBuffer, spawn, kill } = usePtyProcess();

  useEffect(() => {
    // AIツールを起動
    spawn('bunx', ['@anthropic-ai/claude-code@latest'], {
      cwd: session.worktreePath,
    });
  }, []);

  useInput((input, key) => {
    // Ctrl+C: 中断
    if (key.ctrl && input === 'c') {
      kill();
      onBack();
      return;
    }

    // 通常の入力をPTYに転送
    ptyProcess?.write(input);
  });

  return (
    <Box flexDirection="column">
      {/* ヘッダー */}
      <Text bold>{session.tool} - {session.branch}</Text>

      {/* 出力 */}
      {outputBuffer.map(line => (
        <Text key={line.id}>{line.content}</Text>
      ))}
    </Box>
  );
}
```

### 3.2 App.tsxへの統合

**ファイル**: `src/ui/components/App.tsx`

```typescript
import { TerminalScreen } from './screens/TerminalScreen.js';

// ...

const handleModeSelect = useCallback((result: { mode: ExecutionMode; skipPermissions: boolean }) => {
  // Ink UIを終了せず、TerminalScreenに遷移
  if (selectedBranch && selectedTool) {
    setTerminalSession({
      id: uuid(),
      branch: selectedBranch,
      tool: selectedTool,
      mode: result.mode,
      worktreePath: getWorktreePath(selectedBranch),
      startTime: new Date(),
      skipPermissions: result.skipPermissions,
    });
    navigateTo('terminal');
  }
}, [selectedBranch, selectedTool, navigateTo]);

// ...

const renderScreen = () => {
  switch (currentScreen) {
    // ...
    case 'terminal':
      return terminalSession ? (
        <TerminalScreen
          session={terminalSession}
          onBack={() => {
            setTerminalSession(null);
            navigateTo('branch-list');
          }}
        />
      ) : null;
    // ...
  }
};
```

## 4. PTYマネージャーの使用例

### 4.1 基本的な使い方

**ファイル**: `src/pty/PtyManager.ts`

```typescript
import * as pty from 'node-pty';
import { IPty } from 'node-pty';

export class PtyManager {
  private pty: IPty | null = null;

  /**
   * PTYプロセスを起動
   */
  spawn(command: string, args: string[], options: {
    cwd?: string;
    env?: Record<string, string>;
  } = {}): IPty {
    this.pty = pty.spawn(command, args, {
      name: 'xterm-256color',
      cols: 80,
      rows: 30,
      cwd: options.cwd || process.cwd(),
      env: { ...process.env, ...options.env },
    });

    return this.pty;
  }

  /**
   * データを書き込む（ユーザー入力）
   */
  write(data: string): void {
    if (this.pty) {
      this.pty.write(data);
    }
  }

  /**
   * プロセスを終了
   */
  kill(): void {
    if (this.pty) {
      this.pty.kill();
      this.pty = null;
    }
  }

  /**
   * 一時停止（Unix系のみ）
   */
  pause(): void {
    if (this.pty && process.platform !== 'win32') {
      process.kill(this.pty.pid, 'SIGSTOP');
    }
  }

  /**
   * 再開（Unix系のみ）
   */
  resume(): void {
    if (this.pty && process.platform !== 'win32') {
      process.kill(this.pty.pid, 'SIGCONT');
    }
  }
}
```

### 4.2 React フックでの使用

**ファイル**: `src/ui/hooks/usePtyProcess.ts`

```typescript
import { useState, useCallback } from 'react';
import { PtyManager } from '../../pty/PtyManager.js';

export function usePtyProcess() {
  const [ptyManager] = useState(() => new PtyManager());
  const [outputBuffer, setOutputBuffer] = useState<string[]>([]);

  const spawn = useCallback((command: string, args: string[], options: any) => {
    const pty = ptyManager.spawn(command, args, options);

    pty.onData((data: string) => {
      setOutputBuffer(prev => [...prev, data]);
    });

    pty.onExit(({ exitCode }) => {
      console.log('PTY exited with code:', exitCode);
    });

    return pty;
  }, [ptyManager]);

  return {
    spawn,
    write: (data: string) => ptyManager.write(data),
    kill: () => ptyManager.kill(),
    pause: () => ptyManager.pause(),
    resume: () => ptyManager.resume(),
    outputBuffer,
  };
}
```

## 5. トラブルシューティング

### 5.1 node-ptyのビルドエラー

**症状**: `bun add node-pty` でエラーが発生

**原因**: ネイティブモジュールのコンパイルに必要なツールが不足

**解決策** (macOS):
```bash
# Xcode Command Line Toolsをインストール
xcode-select --install
```

**解決策** (Linux):
```bash
# ビルドツールをインストール
sudo apt-get install build-essential
```

**解決策** (Windows):
```bash
# WSLを使用するか、Visual Studio Build Toolsをインストール
```

### 5.2 PTYが起動しない

**症状**: `spawn()` 実行時にエラー

**確認項目**:
1. コマンドが存在するか: `which bunx`
2. 作業ディレクトリが存在するか: `ls /path/to/worktree`
3. パーミッションがあるか

**デバッグ**:
```typescript
try {
  const pty = ptyManager.spawn('bunx', ['@anthropic-ai/claude-code@latest']);
} catch (error) {
  console.error('PTY spawn error:', error);
}
```

### 5.3 出力が表示されない

**症状**: PTYは起動するが出力が表示されない

**原因**: React stateの更新が遅い、または出力がバッファリングされている

**解決策**: デバッグログを追加
```typescript
pty.onData((data: string) => {
  console.log('PTY data:', data);
  setOutputBuffer(prev => [...prev, data]);
});
```

### 5.4 キー入力が反映されない

**症状**: キーを押しても何も起こらない

**確認項目**:
1. `useInput()` フックが正しく設定されているか
2. `ptyManager.write()` が呼ばれているか

**デバッグ**:
```typescript
useInput((input, key) => {
  console.log('Key pressed:', { input, key });
  ptyManager.write(input);
});
```

### 5.5 Ctrl+Zが動作しない（Windows）

**症状**: Ctrl+Zを押してもプロセスが一時停止しない

**原因**: WindowsはSIGSTOP/SIGCONTをサポートしていない

**解決策**: Windows環境では一時停止機能を無効化
```typescript
// フッターのアクションを条件分岐
const footerActions = [
  { key: 'Ctrl+C', description: 'Interrupt' },
  ...(process.platform !== 'win32' ? [
    { key: 'Ctrl+Z', description: 'Pause/Resume' }
  ] : []),
  { key: 'Ctrl+S', description: 'Save Log' },
];
```

## 6. デバッグヒント

### 6.1 PTYの動作確認

```bash
# 簡単なコマンドでPTYをテスト
const pty = ptyManager.spawn('echo', ['hello', 'world']);
pty.onData(data => console.log(data));
// 期待: "hello world\n"
```

### 6.2 ログの有効化

```typescript
// PtyManager.tsにログを追加
spawn(command: string, args: string[], options: any): IPty {
  console.log('[PTY] Spawning:', { command, args, cwd: options.cwd });
  const pty = pty.spawn(command, args, { /* ... */ });

  pty.onData(data => {
    console.log('[PTY] Data:', data);
  });

  pty.onExit(({ exitCode }) => {
    console.log('[PTY] Exit:', exitCode);
  });

  return pty;
}
```

### 6.3 Ink UIのデバッグ

```bash
# ink-devtoolsを使用（オプション）
bun add -d ink-devtools

# コンポーネントにDevToolsを追加
import { render } from 'ink';
import DevTools from 'ink-devtools';

render(
  <>
    <App />
    <DevTools />
  </>
);
```

## 7. 参考資料

- [node-pty公式ドキュメント](https://github.com/microsoft/node-pty)
- [Ink公式ドキュメント](https://github.com/vadimdemedes/ink)
- [Vitest公式ドキュメント](https://vitest.dev/)
- [既存のScreenコンポーネント実装](../../src/ui/components/screens/)

## 8. 次のステップ

✅ **クイックスタートガイド完了**

⏭️ **/speckit.tasks**: 実装タスクの生成

⏭️ **/speckit.implement**: TDD実装の開始
