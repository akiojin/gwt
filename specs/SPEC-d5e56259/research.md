# 調査結果: Web UI機能の追加

**日付**: 2025-11-10
**仕様ID**: SPEC-d5e56259
**関連ドキュメント**: [spec.md](./spec.md), [plan.md](./plan.md)

## 1. 既存コードベース分析

### 技術スタック

**言語/ランタイム**:
- TypeScript 5.8.x（厳格モード、exactOptionalPropertyTypes有効）
- Bun 1.0+（ローカル開発・実行）
- pnpm（CI/CD・Docker環境）

**UIフレームワーク**:
- React 19.2.0
- Ink 6.3.1（Terminal UI - CLIで使用）
- ink-select-input 6.2.0、ink-text-input 6.0.0

**コア依存関係**:
- execa 9.6.0（プロセス実行・Git操作）
- chalk 5.4.1（色付き出力）
- string-width 7.2.0（文字幅計算）

**テスト**:
- Vitest 4.0.8
- @testing-library/react 16.3.0
- happy-dom 20.0.8（DOM環境）
- ink-testing-library 4.0.0

**ビルド**:
- TypeScript Compiler（tsc）
- エントリーポイント: bin/claude-worktree.js → dist/index.js

### アーキテクチャパターン

#### コア層（`src/`）

**Git操作（git.ts）**:
- `isGitRepository()`: Gitリポジトリチェック
- `getRepositoryRoot()`: ルート取得
- `getAllBranches()`: ブランチ一覧取得
- `fetchAllRemotes()`: リモート更新
- `getBranchDivergenceStatuses()`: ブランチ差分確認
- 実装: execaでgitコマンド実行、出力をパース

**Worktree管理（worktree.ts）**:
- `worktreeExists()`: 既存worktree検索
- `createWorktree()`: 新規作成
- `removeWorktree()`: 削除
- `getMergedPRWorktrees()`: マージ済みクリーンアップ対象取得
- 実装: `git worktree`コマンドラッパー

**AI Tool起動（claude.ts/codex.ts/launcher.ts）**:
- `launchClaudeCode()`: Claude Code起動（normal/continue/resume）
- `launchCodexCLI()`: Codex CLI起動
- `launchCustomAITool()`: カスタムツール起動
- 実装: execaで`stdio: "inherit"`、`/dev/tty`を使用してInk UIと共存

**サービス層（services/）**:
- `WorktreeOrchestrator`: Worktree存在確認と作成を統合
- `installDependencies()`: 依存関係インストール
- 依存性注入対応（テスト容易性）

**リポジトリ層（repositories/）**:
- `SessionRepository`: セッション履歴の永続化
- `ConfigRepository`: 設定ファイル（~/.claude-worktree/）管理

#### UI層（`src/ui/`）

**画面構造**:
- `App.tsx`: トップレベル、画面遷移管理（useScreenState）
- `BranchListScreen`: ブランチ一覧（メイン画面）
- `AIToolSelectorScreen`: AIツール選択
- `ExecutionModeSelectorScreen`: 実行モード選択
- `WorktreeManagerScreen`: Worktree管理
- `PRCleanupScreen`: PRクリーンアップ

**Hooks**:
- `useGitData`: Gitデータ取得・管理（branches, worktrees, remotes）
- `useScreenState`: 画面遷移状態管理（戻る/進む）
- `useTerminalSize`: ターミナルサイズ検出
- `useBatchMerge`: バッチマージ処理

**共通コンポーネント**:
- `Select`: 選択リスト（ink-select-input）
- `Input`: テキスト入力（ink-text-input）
- `Confirm`: 確認ダイアログ
- `LoadingIndicator`: ローディング表示
- `ErrorBoundary`: エラー境界

#### ターミナル制御（`src/utils/terminal.ts`）

**createChildStdio()**:
- Ink UIがstdinを占有するため、`/dev/tty`から直接ファイルディスクリプタを開く
- AI Tool起動時にInkのrawモードを解除
- クリーンアップでfdをクローズ

**exitRawMode()**:
- ターミナルのrawモードを解除
- Inkが残したイベントリスナーを削除

### 統合ポイント

#### エントリーポイント（`src/index.ts`）

**main()関数**:
1. コマンドライン引数パース（`-h/--help`, `-v/--version`）
2. Gitリポジトリチェック
3. `runInteractiveLoop()`: Ink UI起動 → ワークフロー → AI Tool起動のサイクル

**runInteractiveLoop()**:
1. Ink UIで選択（ブランチ、ツール、モード）
2. Ink UI終了
3. ターミナルクリーンアップ（exitRawMode, removeAllListeners）
4. AI Tool起動（execa、await）
5. セッション保存
6. ループ継続

#### Web UIとの統合方針

**既存コードの再利用**:
- **git.ts、worktree.ts**: REST API経由で公開（変更不要）
- **WorktreeOrchestrator**: そのまま再利用
- **useGitData、useScreenStateのパターン**: Web UIで応用

**分離が必要な箇所**:
- **Ink UI**: src/cli/ui/に移動
- **ターミナル制御**: Web版では不要（PTYで代替）
- **エントリーポイント**: CLI/Web分岐ロジック追加

## 2. 技術スタック決定

### 選定結果

#### バックエンド

**Fastify 5.x**
- **選定理由**: 高速（最大65k req/s）、TypeScript完全サポート、プラグインエコシステム豊富
- **代替案**: Express（遅い）、Hono（新しすぎ）
- **バージョン**: 5.1.x（最新安定版）

**@fastify/websocket 11.x**
- **選定理由**: Fastify公式プラグイン、ws（業界標準）ベース、型安全
- **代替案**: socket.io（重い、オーバースペック）
- **バージョン**: 11.1.x

**@fastify/static 8.x**
- **選定理由**: 静的ファイル配信、Fastify公式
- **用途**: フロントエンドビルド成果物（HTML/CSS/JS）の配信

**node-pty 1.1.x**
- **選定理由**: 業界標準（VS Code使用）、クロスプラットフォーム、ANSI対応
- **代替案**: node-pty-prebuilt-multiarch（ビルド済みバイナリ、メンテナンス不明）
- **Windows対応**: ConPTY使用（Windows 10 1809以降）
- **バージョン**: 1.1.0-beta28（最新）

#### フロントエンド

**React 19 + TypeScript 5.8**
- **選定理由**: 既存Ink UIと同じバージョン、知識再利用可能
- **新機能**: React Server Components（不使用）、Actions（不使用）

**Vite 6.x**
- **選定理由**: HMR高速、Bun互換、React 19対応、軽量
- **代替案**: esbuild（プラグイン不足）、Webpack（遅い）
- **プラグイン**: @vitejs/plugin-react-swc（高速コンパイル）

**xterm.js 5.5.x**
- **選定理由**: ANSI escape codes完全対応、VS Code Web版使用実績、アクティブメンテナンス
- **アドオン**: xterm-addon-fit（リサイズ対応）
- **代替案**: term.js（開発停止）、hterm（Chromeのみ）

**TanStack Query 5.x**
- **選定理由**: サーバー状態管理、自動キャッシング、楽観的更新、エラーリトライ
- **用途**: REST API呼び出し、ブランチ/Worktree一覧取得

**Zustand 5.x**
- **選定理由**: 軽量（<1KB）、Redux代替、TypeScript完全対応
- **用途**: クライアント状態（選択中のブランチ、画面遷移）

**shadcn/ui**
- **選定理由**: Radix UI + Tailwind CSS、アクセシビリティ対応、カスタマイズ容易
- **代替案**: Chakra UI（重い）、Material UI（重い）
- **必要コンポーネント**: Button, Select, Input, Dialog, Tabs

### 決定の根拠

#### PTY + WebSocket + xterm.jsの組み合わせ

**技術的実現可能性**: ✅ 実証済み
- VS Code Web版が同じスタックを使用
- Gotty（Terminal共有ツール）も同じ構成
- コミュニティで広く使用

**アーキテクチャ**:
```
Browser (xterm.js)
    ↕ WebSocket
Fastify Server
    ↕ node-pty
Claude Code / Codex CLI
```

**データフロー**:
1. ユーザーがxterm.jsでキー入力
2. WebSocketでサーバーに送信 `{ type: 'input', data: 'x' }`
3. サーバーがPTYに書き込み `pty.write('x')`
4. PTYからClaude Codeに転送
5. Claude Codeの出力がPTYから読み取り `pty.onData(data => ...)`
6. WebSocketでブラウザに送信 `{ type: 'output', data: '...' }`
7. xterm.jsが描画 `term.write(data)`

#### React 19 + Viteの組み合わせ

**既存知識の再利用**:
- Ink UIもReactベース（JSX構文、hooks）
- useStateの代わりにZustand、useEffectはそのまま
- コンポーネント設計パターンが同じ

**開発体験**:
- HMR（Hot Module Replacement）で即座に反映
- Bun互換（`bun run dev`）
- TypeScript型チェック統合

## 3. 制約と依存関係

### 技術制約

#### PTY制約

**Windows**:
- ConPTY使用（Windows 10 1809以降）
- MSYS2/Cygwinは未サポート（node-pty制限）
- 一部ANSI escape codesが未対応（ConPTY制限）

**Unix（macOS/Linux）**:
- forkpty使用（POSIX標準）
- 完全なANSI対応
- pty デバイスファイル（/dev/pts/）が必要

**Docker**:
- `/dev/pts`をマウント必須: `-v /dev/pts:/dev/pts`
- `--privileged`または`--cap-add=SYS_ADMIN`

#### WebSocket制約

**接続数上限**:
- 理論上: 無制限
- 実用上: 20接続（PTY 10 + ログストリーム 10）
- OS制限: ファイルディスクリプタ上限（ulimit -n）

**メッセージサイズ**:
- 最大: 1MB（@fastify/websocketデフォルト）
- 推奨: 1KB以下（ターミナル出力は小チャンク）

#### Git操作制約

**`.git/index.lock`競合**:
- 同時git操作で`fatal: Unable to create '.git/index.lock': File exists`
- 対策: p-queue（キューイング）でシリアライズ

**ブランチ取得パフォーマンス**:
- 1000ブランチ: `git branch -a` 約1秒
- 10000ブランチ: 約10秒
- 対策: フロントエンドでページネーション、仮想スクロール

### 互換性要件

#### 既存CLI互換性

**コマンド分岐**:
```typescript
// src/index.ts
if (process.argv.includes('--web') || process.argv.includes('serve')) {
  await startWebServer();
} else {
  await runCLI();
}
```

**設定ファイル共有**:
- `~/.config/claude-worktree/config.json`（既存）
- `~/.config/claude-worktree/tools.json`（既存）
- Web UIで変更 → CLI即座に反映

**セッション履歴共有**:
- `~/.config/claude-worktree/sessions.json`
- CLI/Web両方で同じ履歴参照

#### Bun互換性

**開発コマンド**:
- `bun install`: 依存インストール
- `bun run build`: CLIビルド
- `bun run build:web`: Web UIビルド
- `bun run dev:web`: Web UI開発サーバー
- `bunx .`: CLI実行
- `bunx . serve`: Web サーバー起動

**CI/CD（pnpm）**:
- `.github/workflows/`でpnpm使用
- ハードリンクでnode_modules効率化

## 4. 推奨事項

### 実装アプローチ

#### Phase 1: 最小限のPOC（2-3日）

**目的**: PTY + WebSocket + xterm.jsの技術検証

**成果物**:
- Fastifyサーバー起動（`bun run dev:server`）
- PTYでshellを起動
- xterm.jsでブラウザに表示
- キーボード入力が動作

**実装**:
```typescript
// web/server/index.ts
import Fastify from 'fastify';
import websocket from '@fastify/websocket';
import pty from 'node-pty';

const app = Fastify();
app.register(websocket);

app.get('/ws', { websocket: true }, (socket) => {
  const shell = pty.spawn('bash', [], { cols: 80, rows: 30 });
  shell.onData(data => socket.send(JSON.stringify({ type: 'output', data })));
  socket.on('message', msg => {
    const { type, data } = JSON.parse(msg.toString());
    if (type === 'input') shell.write(data);
  });
});

app.listen({ port: 3000 });
```

```typescript
// web/client/src/App.tsx
import { Terminal } from 'xterm';
import { useEffect, useRef } from 'react';

export default function App() {
  const termRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const term = new Terminal();
    term.open(termRef.current!);
    const ws = new WebSocket('ws://localhost:3000/ws');
    ws.onmessage = e => {
      const { type, data } = JSON.parse(e.data);
      if (type === 'output') term.write(data);
    };
    term.onData(data => ws.send(JSON.stringify({ type: 'input', data })));
  }, []);
  return <div ref={termRef} />;
}
```

#### Phase 2: Git/Worktree REST API（3-4日）

**目的**: 既存git.ts、worktree.tsをREST API化

**成果物**:
- `GET /api/branches`: ブランチ一覧
- `POST /api/worktrees`: Worktree作成
- `DELETE /api/worktrees/:path`: Worktree削除
- エラーハンドリング（ZodでValidation）

**実装**:
```typescript
// web/server/routes/branches.ts
import { getAllBranches } from '../../../core/git';

export async function branchesRoute(app: FastifyInstance) {
  app.get('/api/branches', async (req, reply) => {
    try {
      const branches = await getAllBranches(repoRoot);
      return { success: true, data: branches };
    } catch (error) {
      reply.code(500);
      return { success: false, error: error.message };
    }
  });
}
```

#### Phase 3: フルUI実装（5-7日）

**目的**: Ink UIと同等の機能をWeb UIで実現

**成果物**:
- ブランチ一覧画面（検索・フィルター）
- AI Tool選択画面
- ターミナル画面（xterm.js統合）
- Worktree管理画面
- 設定管理画面

**実装**:
- shadcn/uiコンポーネント使用
- TanStack Queryでデータフェッチ
- Zustandで画面遷移管理

#### Phase 4: 追加機能（3-5日）

**目的**: UX向上機能

**成果物**:
- セッション管理（再接続、履歴表示）
- GitHub PR統合（マージ済みブランチ表示）
- ターミナル高度機能（ログ保存、検索）

#### Phase 5: テスト・ドキュメント（2-3日）

**目的**: 品質保証とドキュメント

**成果物**:
- ユニットテスト（Vitest）
- E2Eテスト（Playwright）
- README.md更新
- API仕様書（OpenAPI）

### リスク軽減策

#### PTYゾンビ化

**問題**: WebSocket切断時にPTYプロセスが残る

**対策**:
```typescript
socket.on('close', () => {
  ptyProcess.kill('SIGTERM');
  setTimeout(() => ptyProcess.kill('SIGKILL'), 5000); // 5秒後に強制終了
});
```

#### Gitロック競合

**問題**: 同時git操作で`.git/index.lock`エラー

**対策**:
```typescript
import PQueue from 'p-queue';
const gitQueue = new PQueue({ concurrency: 1 });

export async function getAllBranches(repoRoot: string) {
  return gitQueue.add(() => getAllBranchesImpl(repoRoot));
}
```

#### バッファオーバーフロー

**問題**: PTY出力が大量でWebSocketバッファあふれ

**対策**:
```typescript
let buffer = '';
ptyProcess.onData(data => {
  buffer += data;
  if (buffer.length > 1024) {
    socket.send(JSON.stringify({ type: 'output', data: buffer }));
    buffer = '';
  }
});
```

#### メモリリーク

**問題**: xterm.jsインスタンスがdispose未実行

**対策**:
```typescript
useEffect(() => {
  const term = new Terminal();
  term.open(termRef.current!);
  return () => term.dispose(); // クリーンアップ
}, []);
```

#### 型不一致

**問題**: REST APIレスポンスとフロントエンドの型が不一致

**対策**:
```typescript
// src/types/api.ts（共通型定義）
export interface Branch {
  name: string;
  type: 'local' | 'remote';
  worktreePath: string | null;
}

// バックエンド
const branches: Branch[] = await getAllBranches(repoRoot);

// フロントエンド
const { data } = useQuery<Branch[]>({ queryKey: ['branches'], queryFn: fetchBranches });
```

## 5. 次のステップ

1. ✅ Phase 0完了: 調査と技術スタック決定
2. ⏭️ Phase 1実行: data-model.md, contracts/, quickstart.md生成
3. ⏭️ エージェントコンテキスト更新
4. ⏭️ `/speckit.tasks` 実行: tasks.md生成
5. ⏭️ `/speckit.implement` 実行: TDD開始
