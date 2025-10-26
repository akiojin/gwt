# Phase 0: 調査結果

**仕様ID**: `SPEC-6d501fd0` | **日付**: 2025-01-26
**目的**: 技術スタックの決定と既存コードパターンの理解

## 1. 既存のコードベース分析

### 1.1 Ink UI画面実装パターン

**調査結果**:
既存のScreenコンポーネント（`BranchListScreen.tsx`, `AIToolSelectorScreen.tsx`等）の実装パターンを分析しました。

**共通パターン**:
- Reactコンポーネントとして実装
- `useInput()` フックでキーボード入力を処理
- `useTerminalSize()` フックで画面サイズを取得
- Header + Content + Footer の3層構造
- propsで`onBack`, `onSelect`等のコールバックを受け取る

**例**（BranchListScreen）:
```typescript
export function BranchListScreen({
  branches, onSelect, onNavigate, onQuit
}: BranchListScreenProps) {
  const { rows } = useTerminalSize();

  useInput((input, key) => {
    if (input === 'q') onQuit();
    // ...
  });

  return (
    <Box flexDirection="column" height={rows}>
      <Header title="Branch List" />
      <Box flexGrow={1}>{/* Content */}</Box>
      <Footer actions={[...]} />
    </Box>
  );
}
```

**TerminalScreenへの適用**:
- 同じパターンを踏襲
- useInputで特殊キー（Ctrl+C, Ctrl+Z, Ctrl+S, F11）を処理
- 通常のキー入力はPTYに転送

### 1.2 AIツール起動方法

**現在の実装**（`src/claude.ts`, `src/codex.ts`）:
```typescript
// execa を使用して stdio: "inherit" で起動
await execa("bunx", [CLAUDE_CLI_PACKAGE, ...args], {
  cwd: worktreePath,
  stdio: "inherit",  // <- 直接ターミナルに出力
  shell: true
});
```

**問題点**:
- `stdio: "inherit"` はInk UIを完全にバイパスする
- Ink UIの制御下でAIツールを実行できない
- 出力をキャプチャできない（ログ保存不可）

**解決策**:
- PTYを使用してAIツールを起動
- PTYの出力をInk UIで表示
- ユーザー入力をPTYに転送

### 1.3 画面遷移フロー

**現在のフロー**（`useScreenState` フック使用）:
```
BranchListScreen
  → AIToolSelectorScreen
    → ExecutionModeSelectorScreen
      → [Ink UI終了] → AIツール起動 (stdio: inherit)
```

**新しいフロー**:
```
BranchListScreen
  → AIToolSelectorScreen
    → ExecutionModeSelectorScreen
      → TerminalScreen (新規)
        → [AIツール実行] (PTY経由)
      → BranchListScreen (終了後)
```

**変更点**:
- `App.tsx` の `handleModeSelect()` でInk UIを終了しない
- TerminalScreenに遷移してPTY経由でAIツールを起動
- AIツール終了後、`navigateTo('branch-list')` で戻る

### 1.4 テストパターン

**既存のテスト**（vitest + ink-testing-library）:
```typescript
import { render } from 'ink-testing-library';

test('renders correctly', () => {
  const { lastFrame } = render(<BranchListScreen {...props} />);
  expect(lastFrame()).toContain('Branch List');
});
```

**TerminalScreenのテスト戦略**:
- コンポーネントのレンダリングテスト（ink-testing-library）
- PTYマネージャーのモック化
- キーボード入力のシミュレーション
- 統合テストで実際のAIツール起動を検証

## 2. 技術的決定

### 2.1 PTY（疑似端末）ライブラリの選択

**決定**: `node-pty` を使用

**理由**:
- Microsoft公式のPTYライブラリ
- VS CodeやGitHub Codespacesでも使用されている実績
- クロスプラットフォーム対応（Windows, macOS, Linux）
- Active maintenanceされている（最終更新: 2024年）

**代替案の検討**:
- `node-pty-prebuilt`: ビルド済みバイナリを提供するが、最新版への追従が遅い
- `pty.js`: 非推奨（node-ptyに移行推奨）
- 自前実装: 複雑すぎる

**Bun互換性**:
- node-ptyはネイティブモジュール（C++）のため、Bunでコンパイル可能か要検証
- Bunはnode-gyp互換のビルドシステムを提供
- 最悪の場合、Node.js環境でのみ動作させるフォールバックを用意

**インストール**:
```bash
bun add node-pty
bun add -d @types/node-pty
```

### 2.2 Ink UIでのraw入力処理

**決定**: `useInput()` フックとプロセスstdinの直接制御を組み合わせる

**調査結果**:
- `useInput()` は文字単位で入力を受け取る
- 特殊キー（Ctrl+C, Ctrl+Z等）はInkが処理する前にインターセプト可能
- 通常の文字入力はPTYに転送

**実装アプローチ**:
```typescript
useInput((input, key) => {
  // 特殊キーを処理
  if (key.ctrl && input === 'c') {
    // Ctrl+C: プロセス中断
    ptyManager.kill();
    onBack();
    return;
  }

  if (key.ctrl && input === 'z') {
    // Ctrl+Z: 一時停止/再開
    if (isPaused) {
      ptyManager.resume();
    } else {
      ptyManager.pause();
    }
    return;
  }

  // 通常の入力はPTYに転送
  ptyManager.write(input);
});
```

**制約**:
- InkはReact Reconcilerを使用しているため、レンダリングサイクルに影響される
- 高頻度の入力（キーリピート）でも遅延が発生しないよう最適化が必要

### 2.3 プロセス制御（SIGSTOP/SIGCONT）

**決定**: POSIXシグナルを使用（Unix系）、Windows向けにフォールバック実装

**Unix系（macOS, Linux）**:
```typescript
// 一時停止
process.kill(ptyProcess.pid, 'SIGSTOP');

// 再開
process.kill(ptyProcess.pid, 'SIGCONT');
```

**Windows**:
- SIGSTOPはサポートされていない
- 代替案: プロセスのサスペンド/レジュームAPIを使用（Windows専用）
- または、一時停止機能をUnix系のみで提供

**実装方針**:
- プラットフォームを `process.platform` で判定
- Windows では一時停止機能を無効化（フッターに表示しない）

### 2.4 ログ保存機能

**決定**: ストリーミングバッファリング + ファイル書き込み

**実装アプローチ**:
1. PTYからの出力を`outputBuffer: string[]`に蓄積
2. Ctrl+S押下時、バッファを`.logs/{timestamp}-{tool}-{branch}.log`に保存
3. ファイル保存にはNode.js標準の`fs.promises.writeFile()`を使用

**ログファイル命名規則**:
```
.logs/2025-01-26T10-30-45-claude-code-feature-terminal.log
```

**エラーハンドリング**:
- ディスク容量不足: エラーメッセージ表示
- パーミッションエラー: エラーメッセージ表示
- ディレクトリが存在しない: 自動作成（`fs.promises.mkdir(dirPath, { recursive: true })`）

## 3. 制約と依存関係

### 3.1 既存コードの変更範囲

**最小限の変更**:
- `src/index.ts`: `handleAIToolWorkflow()` を削除、Ink UI内で完結させる
- `src/ui/components/App.tsx`: `handleModeSelect()` でTerminalScreenに遷移
- `src/claude.ts`, `src/codex.ts`: PTY対応関数を追加（既存関数は維持）

**新規ファイル**:
- `src/pty/PtyManager.ts`
- `src/pty/types.ts`
- `src/ui/hooks/usePtyProcess.ts`
- `src/ui/components/screens/TerminalScreen.tsx`
- `src/ui/components/parts/TerminalOutput.tsx`

### 3.2 既存テストへの影響

**回帰テスト**:
- すべての既存テストを実行して回帰を確認
- App.tsxのテストを更新（新しい画面遷移フローに対応）

### 3.3 パフォーマンス要件

**目標**:
- キー入力レイテンシ < 100ms
- 出力表示レイテンシ < 200ms

**最適化ポイント**:
- PTY出力をバッチ処理（16ms毎にReact stateを更新）
- 大量の出力時、表示行数を制限（スクロールバッファ）

## 4. 実装推奨事項

### 4.1 Phase 1の優先タスク

1. **PtyManager基本実装**
   - spawn(), kill(), write()
   - データイベントのハンドリング
2. **TerminalScreen基本実装**
   - ヘッダー表示
   - PTY出力の表示
   - 基本的なキー入力処理
3. **App.tsx統合**
   - 画面遷移フローの変更

### 4.2 Phase 2の優先タスク

1. **プロセス制御機能**
   - Ctrl+C（中断）
   - Ctrl+Z（一時停止/再開 - Unix系のみ）
2. **ログ保存機能**
   - Ctrl+S（ログ保存）
3. **全画面モード**
   - F11（全画面切替）

### 4.3 テスト戦略

**TDDアプローチ**:
1. テストを先に書く
2. テストが失敗することを確認
3. 実装してテストをパスさせる
4. リファクタリング

**重点テスト領域**:
- PtyManagerのライフサイクル
- エラーハンドリング（起動失敗、異常終了）
- プラットフォーム固有の動作

## 5. リスク評価

### 5.1 高リスク

**node-ptyのBun互換性**:
- **状況**: Bunでのnode-ptyビルドが未検証
- **対策**: 早期にプロトタイプを作成して検証
- **フォールバック**: Node.js環境でのみ動作させる

### 5.2 中リスク

**Ink UIのパフォーマンス**:
- **状況**: 高頻度の出力でReactレンダリングが追いつかない可能性
- **対策**: バッチ処理と表示行数制限

**Windows対応**:
- **状況**: SIGSTOP/SIGCONTが使えない
- **対策**: 一時停止機能をUnix系のみで提供

### 5.3 低リスク

**既存コードへの影響**:
- **状況**: 変更範囲は限定的
- **対策**: 既存テストで回帰を確認

## 6. 次のステップ

✅ **調査完了**: すべての技術的決定が完了しました

⏭️ **Phase 1**: 設計ドキュメント（data-model.md, quickstart.md）の生成

**推奨アクション**:
1. node-ptyをインストールしてBunでのビルドを検証
2. 簡単なPTYプロトタイプを作成してInk UIでの表示を確認
3. Phase 1の設計ドキュメントを作成
