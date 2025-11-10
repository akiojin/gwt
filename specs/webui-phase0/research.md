# 調査結果: Web UI機能の追加

## 1. 既存コードベース分析

### 1.1 技術スタック

**言語とランタイム**:
- TypeScript 5.8.x (厳格な型チェック: `strict: true`, `noImplicitAny: true`, `noUncheckedIndexedAccess: true`)
- Bun 1.0+ (ローカル開発・実行環境)
- pnpm (CI/CD環境でのハードリンク効率化)
- Node.js互換: ESM形式 (`"type": "module"`)

**UIフレームワーク（既存CLI）**:
- React 19.2.0 (最新安定版)
- Ink 6.3.1 (Terminal UI)
- ink-select-input 6.2.0 (リスト選択UI)
- ink-text-input 6.0.0 (テキスト入力UI)

**ビルドとテスト**:
- TypeScript Compiler (tsc) - ビルドツール
- Vitest 4.0.8 - テストフレームワーク
- happy-dom 20.0.8 - DOM環境シミュレーション
- @testing-library/react 16.3.0 - React コンポーネントテスト
- @vitest/coverage-v8 4.0.8 - カバレッジ測定

**その他の依存関係**:
- execa 9.6.0 - プロセス実行（Git操作に使用）
- chalk 5.4.1 - ターミナル色付け
- string-width 7.2.0 - ANSI文字列幅計算

**ビルド設定 (tsconfig.json)**:
```json
{
  "target": "ES2022",
  "module": "ESNext",
  "moduleResolution": "bundler",
  "jsx": "react",
  "outDir": "./dist",
  "rootDir": "./src",
  "strict": true
}
```

**テスト設定 (vitest.config.ts)**:
- globals: true (グローバルテストAPI)
- environment: happy-dom (軽量DOM環境)
- setupFiles: vitest.setup.ts
- coverage: v8プロバイダー、30%ライン/50%関数/70%ブランチ閾値

### 1.2 アーキテクチャパターン

#### コア層の構造

**1. エントリーポイント (src/index.ts)**:
```typescript
// 主な責務:
// - コマンドライン引数のパース (-h, -v)
// - Git リポジトリ検証
// - メインループ実行 (runInteractiveLoop)

export async function main(): Promise<void>
export async function runInteractiveLoop(
  uiHandler: UIHandler = mainInkUI,
  workflowHandler: WorkflowHandler = handleAIToolWorkflow
): Promise<void>
export async function handleAIToolWorkflow(
  selectionResult: SelectionResult
): Promise<void>
```

**設計パターン**: Dependency Injection
- `runInteractiveLoop` はハンドラーを引数で受け取り、テスタビリティを確保
- UI層とワークフロー層を分離

**2. Git操作層 (src/git.ts)**:
```typescript
// 主な機能:
// - リポジトリ検証 (isGitRepository, getRepositoryRoot)
// - ブランチ管理 (getAllBranches, createBranch, deleteBranch, branchExists)
// - Worktree状態確認 (hasUncommittedChanges, hasUnpushedCommits)
// - リモート同期 (fetchAllRemotes, pullFastForward)
// - マージ操作 (mergeFromBranch, hasMergeConflict, abortMerge)
// - 差分確認 (getBranchDivergenceStatuses)

export class GitError extends Error
export async function isGitRepository(): Promise<boolean>
export async function getAllBranches(): Promise<BranchInfo[]>
export async function hasUncommittedChanges(worktreePath: string): Promise<boolean>
export async function hasUnpushedCommits(worktreePath: string, branch: string): Promise<boolean>
export async function fetchAllRemotes(options?: { cwd?: string }): Promise<void>
export async function pullFastForward(worktreePath: string, remote = "origin"): Promise<void>
export async function getBranchDivergenceStatuses(options?: {
  cwd?: string;
  remote?: string;
  branches?: string[];
}): Promise<BranchDivergenceStatus[]>
```

**エラーハンドリング**: カスタムエラークラス (GitError) で詳細なエラー情報を伝播

**3. Worktree管理層 (src/worktree.ts)**:
```typescript
// 主な機能:
// - Worktree作成・削除 (createWorktree, removeWorktree)
// - Worktree一覧取得 (listAdditionalWorktrees)
// - クリーンアップ候補検出 (getMergedPRWorktrees)
// - 保護ブランチ処理 (isProtectedBranchName, switchToProtectedBranch)

export class WorktreeError extends Error
export interface WorktreeInfo {
  path: string;
  branch: string;
  head: string;
  isAccessible?: boolean;
  invalidReason?: string;
}
export async function createWorktree(config: WorktreeConfig): Promise<void>
export async function listAdditionalWorktrees(): Promise<WorktreeInfo[]>
export async function getMergedPRWorktrees(): Promise<CleanupTarget[]>
export function isProtectedBranchName(branchName: string): boolean
export async function switchToProtectedBranch({
  branchName,
  repoRoot,
  remoteRef
}: {
  branchName: string;
  repoRoot: string;
  remoteRef?: string | null;
}): Promise<"none" | "local" | "remote">
```

**保護ブランチ制約**: `PROTECTED_BRANCHES = ["main", "master", "develop"]` はWorktreeを作成せず、ルートディレクトリで切り替え

**4. AI Tool起動層 (src/claude.ts, src/codex.ts, src/launcher.ts)**:
```typescript
// claude.ts - Claude Code起動
export class ClaudeError extends Error
export async function launchClaudeCode(
  worktreePath: string,
  options: {
    skipPermissions?: boolean;
    mode?: "normal" | "continue" | "resume";
    extraArgs?: string[];
  } = {}
): Promise<void>

// codex.ts - Codex CLI起動
export class CodexError extends Error
export async function launchCodexCLI(
  worktreePath: string,
  options: {
    mode?: "normal" | "continue" | "resume";
    extraArgs?: string[];
    bypassApprovals?: boolean;
  } = {}
): Promise<void>

// launcher.ts - カスタムツール起動
export async function resolveCommand(commandName: string): Promise<string>
export async function launchCustomAITool(
  tool: CustomAITool,
  options: LaunchOptions = {}
): Promise<void>
```

**統一されたインターフェース**: 3つのツールとも `mode` と追加引数をサポート
**ターミナル制御**: `getTerminalStreams()` と `createChildStdio()` で親プロセスとの入出力を継承

**5. サービス層 (src/services/)**:
```typescript
// WorktreeOrchestrator.ts - Worktree作成の統合管理
export class WorktreeOrchestrator {
  async ensureWorktree(
    branch: string,
    repoRoot: string,
    options: EnsureWorktreeOptions = {}
  ): Promise<string>
}

// BatchMergeService.ts - 一括マージ処理
// dependency-installer.ts - 依存関係同期
export async function installDependenciesForWorktree(
  worktreePath: string
): Promise<{ skipped: boolean; manager: string }>
```

**6. リポジトリ層 (src/repositories/)**:
```typescript
// git.repository.ts, worktree.repository.ts, github.repository.ts
// データアクセス層の抽象化（将来的な拡張性）
```

#### UI層の構造 (src/ui/)

**1. メインコンポーネント (src/ui/components/App.tsx)**:
```typescript
export interface SelectionResult {
  branch: string;
  displayName: string;
  branchType: 'local' | 'remote';
  remoteBranch?: string;
  tool: AITool;
  mode: ExecutionMode;
  skipPermissions: boolean;
}

export interface AppProps {
  onExit: (result?: SelectionResult) => void;
  loadingIndicatorDelay?: number;
}

export function App({ onExit, loadingIndicatorDelay = 300 }: AppProps)
```

**設計**: Screenベースのナビゲーション
- `currentScreen` で表示画面を切り替え
- `useScreenState` フックで履歴管理
- 各画面は独立したコンポーネント (BranchListScreen, AIToolSelectorScreen, etc.)

**状態管理**:
```typescript
// 選択状態
const [selectedBranch, setSelectedBranch] = useState<SelectedBranchState | null>(null);
const [selectedTool, setSelectedTool] = useState<AITool | null>(null);

// クリーンアップフィードバック
const [cleanupIndicators, setCleanupIndicators] = useState<Record<string, { icon: string; color?: 'cyan' | 'green' | 'yellow' | 'red' }>>({});
const [cleanupInputLocked, setCleanupInputLocked] = useState(false);
const [cleanupFooterMessage, setCleanupFooterMessage] = useState<{ text: string; color?: 'cyan' | 'green' | 'yellow' | 'red' } | null>(null);
```

**2. カスタムフック (src/ui/hooks/)**:
```typescript
// useGitData.ts - Git データフェッチ
export interface UseGitDataResult {
  branches: BranchInfo[];
  worktrees: GitWorktreeInfo[];
  loading: boolean;
  error: Error | null;
  refresh: () => void;
  lastUpdated: Date | null;
}

export function useGitData(options?: UseGitDataOptions): UseGitDataResult

// useScreenState.ts - 画面ナビゲーション
export interface ScreenStateResult {
  currentScreen: ScreenType;
  navigateTo: (screen: ScreenType) => void;
  goBack: () => void;
  reset: () => void;
}

export function useScreenState(): ScreenStateResult

// useBatchMerge.ts - 一括マージUI
// useTerminalSize.ts - ターミナルサイズ検出
```

**データフロー**: 
1. `useGitData` がGit情報をフェッチ
2. App.tsxで `branches` → `branchItems` に変換 (formatBranchItems)
3. 各Screenコンポーネントにpropsとして渡す

**3. 画面コンポーネント (src/ui/components/screens/)**:
```typescript
// BranchListScreen.tsx - ブランチ一覧
// AIToolSelectorScreen.tsx - AIツール選択
// ExecutionModeSelectorScreen.tsx - 実行モード選択
// BranchCreatorScreen.tsx - ブランチ作成
// WorktreeManagerScreen.tsx - Worktree管理
// PRCleanupScreen.tsx - PR クリーンアップ
// SessionSelectorScreen.tsx - セッション選択
// BranchActionSelectorScreen.tsx - ブランチ操作選択
```

**共通パターン**:
```typescript
interface ScreenProps {
  onBack: () => void;
  onSelect: (item: T) => void;
  version?: string | null;
}
```

**4. 共通UIコンポーネント (src/ui/components/common/)**:
```typescript
// Select.tsx - リスト選択UI (ink-select-inputラッパー)
// Input.tsx - テキスト入力UI (ink-text-inputラッパー)
// Confirm.tsx - 確認ダイアログ
// LoadingIndicator.tsx - ローディング表示
// ErrorBoundary.tsx - エラーバウンダリ
```

**パフォーマンス最適化**: 
- `React.memo` で不要な再レンダリングを防止
- `useMemo` で計算結果をキャッシュ
- 1000+ ブランチでもスムーズに動作

**5. パーツコンポーネント (src/ui/components/parts/)**:
```typescript
// Header.tsx - ヘッダー（統計情報表示）
// Footer.tsx - フッター（キーボードショートカット表示）
// Stats.tsx - 統計情報
// ScrollableList.tsx - スクロール可能なリスト
// ProgressBar.tsx - プログレスバー
// MergeStatusList.tsx - マージ状態リスト
```

#### ターミナル制御 (src/utils/terminal.ts)

**課題**: Ink UIとAIツール（Claude Code/Codex CLI）の入出力の衝突を回避

**解決策**:
```typescript
export interface TerminalStreams {
  stdin: NodeJS.ReadStream;
  stdout: NodeJS.WriteStream;
  stderr: NodeJS.WriteStream;
  stdinFd?: number;
  stdoutFd?: number;
  stderrFd?: number;
  usingFallback: boolean;
  exitRawMode: () => void;
}

export function getTerminalStreams(): TerminalStreams
export function createChildStdio(): ChildStdio
```

**動作**:
1. `process.stdin.isTTY` が true の場合: 通常のストリームを使用
2. Unix/Linuxで `/dev/tty` が利用可能な場合: 専用のFile Descriptorを開く
3. Windowsまたはフォールバック: `process.stdin/stdout/stderr` を使用

**AI Tool起動時の処理**:
```typescript
// src/index.ts - mainInkUI()
const { unmount, waitUntilExit } = render(React.createElement(App, { onExit: ... }), {
  stdin: terminal.stdin,
  stdout: terminal.stdout,
  stderr: terminal.stderr,
});

try {
  await waitUntilExit();
} finally {
  terminal.exitRawMode();
  terminal.stdin.removeAllListeners?.("data");
  terminal.stdin.removeAllListeners?.("keypress");
  terminal.stdin.removeAllListeners?.("readable");
  unmount();
}
```

**重要**: AI Tool起動前にInk UIを完全にクリーンアップすることで、入力の奪い合いを防止

### 1.3 統合ポイント

#### CLI → Web UI の分岐設計

**オプション1: コマンド引数による分岐**
```typescript
// src/index.ts
const args = process.argv.slice(2);
if (args.includes("--web")) {
  await startWebServer();
} else {
  await runInteractiveLoop(); // 既存のInk UI
}
```

**オプション2: 環境変数による分岐**
```typescript
if (process.env.CLAUDE_WORKTREE_MODE === "web") {
  await startWebServer();
} else {
  await runInteractiveLoop();
}
```

**推奨**: オプション1（明示的なコマンドフラグ）
- ユーザーに分かりやすい
- ヘルプメッセージに記載しやすい

#### Web UIでの統合戦略

**共有可能なコア機能**:
- Git操作 (src/git.ts) → REST API経由で呼び出し
- Worktree管理 (src/worktree.ts) → REST API経由で呼び出し
- WorktreeOrchestrator (src/services/WorktreeOrchestrator.ts) → そのまま再利用
- 依存関係インストール (src/services/dependency-installer.ts) → そのまま再利用

**適応が必要な機能**:
- AI Tool起動 (src/claude.ts, src/codex.ts)
  - Web UI: PTYセッションを作成し、WebSocket経由でターミナル出力をストリーミング
  - CLI: `createChildStdio()` で親プロセスに入出力を継承

**新規実装が必要な機能**:
- WebSocketサーバー (PTYセッション管理)
- REST APIサーバー (Git/Worktree操作)
- Reactフロントエンド (xterm.jsベースのターミナルUI)

### 1.4 既存パターンの再利用

#### 1. 状態管理パターン

**Ink UI (既存CLI)**:
```typescript
// useState + useCallbackでローカル状態管理
const [selectedBranch, setSelectedBranch] = useState<SelectedBranchState | null>(null);
const [selectedTool, setSelectedTool] = useState<AITool | null>(null);

const handleSelect = useCallback((item: BranchItem) => {
  setSelectedBranch({ ... });
  navigateTo('branch-action-selector');
}, [navigateTo]);
```

**Web UI (提案)**:
```typescript
// TanStack Query + Zustandでサーバー状態とクライアント状態を分離
// サーバー状態（Git/Worktree情報）
const { data: branches, isLoading, refetch } = useQuery({
  queryKey: ['branches'],
  queryFn: () => fetch('/api/branches').then(r => r.json())
});

// クライアント状態（UI選択状態）
const { selectedBranch, setSelectedBranch } = useBranchSelectionStore();
```

#### 2. エラーハンドリングパターン

**既存CLI**:
```typescript
// カスタムエラークラスで型安全なエラー処理
export class GitError extends Error {
  constructor(message: string, public cause?: unknown) {
    super(message);
    this.name = "GitError";
  }
}

// エラー判定ヘルパー
function isGitRelatedError(error: unknown): boolean {
  return error instanceof GitError || error instanceof WorktreeError;
}

// リトライ可能エラーの判定
function isRecoverableError(error: unknown): boolean {
  return error instanceof GitError || error instanceof WorktreeError || 
         error instanceof CodexError || error instanceof DependencyInstallError;
}
```

**Web UI (提案)**:
```typescript
// 同じエラークラスをREST APIレスポンスとして返す
app.get('/api/branches', async (req, res) => {
  try {
    const branches = await getAllBranches();
    res.json(branches);
  } catch (error) {
    if (error instanceof GitError) {
      res.status(500).json({ error: error.message, type: 'GitError' });
    } else {
      res.status(500).json({ error: 'Unknown error' });
    }
  }
});

// フロントエンド側で同じエラー型を復元
const { data, error } = useQuery({
  queryKey: ['branches'],
  queryFn: async () => {
    const res = await fetch('/api/branches');
    if (!res.ok) {
      const json = await res.json();
      if (json.type === 'GitError') {
        throw new GitError(json.error);
      }
      throw new Error(json.error);
    }
    return res.json();
  }
});
```

#### 3. ワークフローパターン

**既存CLI (src/index.ts - handleAIToolWorkflow)**:
```typescript
async function handleAIToolWorkflow(selectionResult: SelectionResult): Promise<void> {
  // 1. リポジトリルート取得
  const repoRoot = await getRepositoryRoot();
  
  // 2. Worktree確保（存在しない場合は作成）
  const worktreePath = await orchestrator.ensureWorktree(branch, repoRoot, options);
  
  // 3. 依存関係インストール
  await installDependenciesForWorktree(worktreePath);
  
  // 4. リモート更新 + Fast-forward pull
  await fetchAllRemotes({ cwd: repoRoot });
  await pullFastForward(worktreePath);
  
  // 5. 差分確認（リモートとローカルの乖離チェック）
  const divergenceStatuses = await getBranchDivergenceStatuses({ cwd: repoRoot, branches: [branch] });
  if (divergedBranches.length > 0) {
    // 警告表示 + AI Tool起動をスキップ
    return;
  }
  
  // 6. AI Tool起動
  if (tool === "claude-code") {
    await launchClaudeCode(worktreePath, { mode, skipPermissions });
  } else if (tool === "codex-cli") {
    await launchCodexCLI(worktreePath, { mode, bypassApprovals: skipPermissions });
  }
  
  // 7. セッション保存
  await saveSession({ lastWorktreePath: worktreePath, lastBranch: branch, lastUsedTool: tool });
}
```

**Web UI (提案)**:
```typescript
// REST API経由で同じワークフローを実行
POST /api/worktree/launch
{
  "branch": "feature/web-ui",
  "tool": "claude-code",
  "mode": "normal",
  "skipPermissions": false
}

// サーバー側でWorktree準備 → PTYセッション作成 → WebSocket URL返却
Response:
{
  "worktreePath": "/path/to/.worktrees/feature-web-ui",
  "ptySessionId": "abc123",
  "websocketUrl": "ws://localhost:3000/pty/abc123"
}

// フロントエンドでxterm.jsに接続
const terminal = new Terminal();
const socket = new WebSocket(websocketUrl);
socket.onmessage = (event) => {
  terminal.write(event.data);
};
terminal.onData((data) => {
  socket.send(JSON.stringify({ type: 'input', data }));
});
```

## 2. 技術スタック決定

### 2.1 選定結果

#### バックエンド

**Webフレームワーク: Fastify 5.x**
- 理由:
  - 高速（Express.jsの2倍のスループット）
  - TypeScript完全サポート（型安全なルーティング）
  - @fastify/websocketで公式WebSocketサポート
  - 軽量（依存関係が少ない）
- 代替案:
  - Express.js: 最もポピュラーだが、TypeScript対応が弱い
  - Hono: 最新だが、Node.js環境での実績が少ない

**WebSocketライブラリ: @fastify/websocket 11.x**
- 理由:
  - Fastifyの公式プラグイン
  - ws（業界標準）のラッパー
  - 自動接続管理とエラーハンドリング
- 代替案:
  - socket.io: 高機能だが、オーバースペック（再接続・ルーム機能が不要）
  - ws: 低レベルすぎる（Fastify統合を自前で実装する必要がある）

**PTYライブラリ: node-pty 1.1.x**
- 理由:
  - VS Code / GitHub Codespaces で使用実績
  - クロスプラットフォーム（Windows: ConPTY, Unix: forkpty）
  - ANSI完全対応
- 代替案:
  - node-pty-prebuilt-multiarch: ビルド済みバイナリだが、メンテナンスが不安定

**プロセス管理: node-pty（PTY） + execa（Git操作）**
- PTY: AI Tool起動（インタラクティブな入出力が必要）
- execa: Git/Worktree操作（stdout/stderrのみ必要）

#### フロントエンド

**フレームワーク: React 19 + TypeScript 5.8**
- 理由:
  - 既存のInk UIと同じReactを使用（学習コスト削減）
  - React 19の新機能（useOptimisticなど）でリアルタイム更新を最適化
  - TypeScript 5.8は既存プロジェクトと完全互換
- 代替案:
  - Vue.js: 学習曲線が緩やかだが、既存コードベースとの一貫性が失われる
  - Svelte: コンパイル時最適化が優れているが、エコシステムが小さい

**ビルドツール: Vite 6.x**
- 理由:
  - HMR（Hot Module Replacement）が高速
  - React 19完全サポート
  - Bun互換（bunx viteで実行可能）
  - 設定がシンプル（tsconfig.jsonと統合）
- 代替案:
  - esbuild: 高速だが、開発サーバー機能が弱い
  - Webpack: 設定が複雑（tsconfig.jsonとの統合が面倒）

**ターミナルエミュレーター: xterm.js 5.5.x + xterm-addon-fit 0.10.x**
- 理由:
  - VS Code / GitHub Codespaces で使用実績
  - ANSI完全対応（色付け・カーソル移動など）
  - WebSocket統合が容易
  - xterm-addon-fitでリサイズ対応
- 代替案:
  - term.js: 開発停止（xterm.jsに統合された）
  - hterm: Google製だが、Chromeに特化しすぎている

**状態管理: TanStack Query 5.x + Zustand 5.x**
- TanStack Query（サーバー状態）:
  - REST APIのキャッシング・リフレッシュを自動化
  - useMutation でWorktree作成などの変更操作を管理
  - useQuery でブランチ一覧などの読み取り操作を管理
- Zustand（クライアント状態）:
  - 軽量（Redux の 1/10 のバンドルサイズ）
  - TypeScript完全サポート
  - ボイラープレート最小
- 代替案:
  - Redux Toolkit + RTK Query: 高機能だが、学習コストが高い
  - Jotai / Recoil: 原子的状態管理だが、TanStack Queryと組み合わせると複雑化

**UIコンポーネント: shadcn/ui (Radix UI + Tailwind CSS)**
- 理由:
  - コピー&ペーストで導入（依存関係が増えない）
  - Radix UIでアクセシビリティ対応済み
  - Tailwind CSSでカスタマイズ容易
  - 既存のCLIと同じデザイン言語を維持できる
- 代替案:
  - Chakra UI: 便利だが、バンドルサイズが大きい
  - Material UI: デザインが固定されすぎている

#### 共通

**モノレポ構造: なし（単一パッケージ）**
- 理由:
  - プロジェクト規模が小さい（CLI + Web UI で十分）
  - 既存のpackage.jsonを拡張して、webサブディレクトリを追加
  - Bun Workspacesを使わず、シンプルなディレクトリ構造を維持
- 構造:
```
claude-worktree/
├── src/                    # 既存CLI（Ink UI）
│   ├── git.ts             # Git操作（CLI/Web共通）
│   ├── worktree.ts        # Worktree管理（CLI/Web共通）
│   └── ui/                # Ink UIコンポーネント
├── web/                    # Web UI（新規追加）
│   ├── server/            # Fastifyサーバー
│   │   ├── index.ts       # エントリーポイント
│   │   ├── routes/        # REST API
│   │   └── pty/           # PTY管理
│   └── client/            # Reactフロントエンド
│       ├── src/
│       │   ├── main.tsx   # エントリーポイント
│       │   ├── components/
│       │   ├── hooks/
│       │   └── stores/
│       └── index.html
├── package.json
└── tsconfig.json
```

### 2.2 決定の根拠

#### 性能要件

**PTYセッション数**: 最大10同時接続（通常は1-2）
- 根拠: 1人の開発者が同時に複数のAI Toolを起動するケースは稀

**WebSocket接続数**: 最大20同時接続（PTY + ログストリーム）
- 根拠: 1つのPTYセッションに対して、1つのWebSocket接続 + 1つのログ表示用WebSocket

**ブランチ一覧表示**: 1000+ ブランチでも1秒以内
- 根拠: 既存のInk UIが1000+ ブランチでもスムーズに動作（useMemo最適化済み）
- Web UIでも同じ最適化を適用（仮想スクロール + ページネーション）

**Git操作レスポンス**: 2秒以内（fetch/pull/merge）
- 根拠: execa で非同期実行しているため、ブロッキングなし

#### パフォーマンス最適化

**既存CLIの最適化手法（そのまま適用可能）**:
```typescript
// src/ui/components/App.tsx
// 1. useMemoでブランチ一覧の計算をキャッシュ
const branchItems = useMemo(() => {
  return formatBranchItems(visibleBranches, worktreeMap);
}, [branchHash, worktreeHash, visibleBranches, worktrees]);

// 2. React.memoで不要な再レンダリングを防止
export const Select = React.memo(SelectComponent);

// 3. コンテンツベースのハッシュで変更検出
const branchHash = useMemo(
  () => visibleBranches.map((b) => `${b.name}-${b.type}-${b.isCurrent}`).join(','),
  [visibleBranches]
);
```

**Web UIでの追加最適化**:
```typescript
// 1. TanStack Queryのキャッシュを活用
const { data: branches } = useQuery({
  queryKey: ['branches'],
  queryFn: fetchBranches,
  staleTime: 5000, // 5秒間はキャッシュを使用
  gcTime: 60000,   // 1分後にキャッシュをクリア
});

// 2. 仮想スクロール（react-window）で1000+ ブランチに対応
import { FixedSizeList } from 'react-window';

<FixedSizeList
  height={600}
  itemCount={branches.length}
  itemSize={50}
  width="100%"
>
  {({ index, style }) => (
    <BranchItem branch={branches[index]} style={style} />
  )}
</FixedSizeList>
```

#### セキュリティ要件

**認証**: なし（ローカル開発環境のみ）
- 根拠: claude-worktreeはローカル開発ツール（本番環境では使用しない）
- 注意: GitHub Codespaces / Dev Containers では、ポート公開を制限

**WebSocket通信**: 暗号化なし（ws://）
- 根拠: ローカルホストのみ（外部ネットワークに公開しない）
- 将来的な拡張: GitHub Codespaces対応時はwss://に切り替え

**PTYセッション管理**: セッションIDで識別
- セッションID: crypto.randomUUID() で生成
- タイムアウト: 1時間アイドルでセッション削除

#### スケーラビリティ

**単一インスタンス**: 1つのFastifyサーバーで全リクエストを処理
- 根拠: ローカル開発環境では、複数インスタンスは不要

**PTYプロセス数**: システムリソース依存
- 制約: ユーザー単位のプロセス数制限（ulimit -u）
- 推奨: 最大10 PTYセッション（実用上は十分）

**WebSocket接続数**: Fastifyのデフォルト制限（10,000）
- 根拠: ローカル開発では100接続以下

## 3. 制約と依存関係

### 3.1 技術制約

#### PTY制約

**Windowsでの制限**:
- ConPTY (Windows 10 1809+) が必須
- Windows 7/8.1 では動作しない
- node-pty 1.1.x は ConPTY自動検出に対応

**Unix/Linuxでの制限**:
- forkpty() システムコールが必須
- Docker環境では `/dev/pts` のマウントが必要
- WSL2 では問題なく動作

**PTYバッファサイズ**:
- デフォルト: 4096バイト
- 大量の出力（git logなど）では、バッファオーバーフローに注意
- 対策: WebSocketで1KBチャンクに分割して送信

#### WebSocket制約

**接続数上限**:
- ブラウザ: 同一ドメインで最大6接続（Chrome/Edge）
- サーバー: Fastifyのデフォルト制限（10,000接続）
- 実用上の制限: 最大20接続（PTY 10 + ログ 10）

**メッセージサイズ上限**:
- デフォルト: 1MB（@fastify/websocket）
- PTY出力: 通常は数KBなので問題なし
- 大きなファイル操作（git diff大量ファイル）では注意

**接続維持**:
- Ping/Pong フレームで接続維持（30秒間隔）
- タイムアウト: 60秒無応答でクローズ
- 再接続: クライアント側で自動リトライ（3回まで）

#### パフォーマンス制約

**Git操作の並列実行**:
- 問題: 複数のWorktreeで同時に `git fetch` すると、`.git/index.lock` で競合
- 対策: キューイング（p-queue）で1つずつ実行
```typescript
import PQueue from 'p-queue';

const gitQueue = new PQueue({ concurrency: 1 });

async function fetchAllRemotes(repoRoot: string) {
  return gitQueue.add(() => execa('git', ['fetch', '--all'], { cwd: repoRoot }));
}
```

**ブランチ一覧のフェッチ頻度**:
- 問題: 頻繁にフェッチすると、Gitリポジトリがロックされる
- 対策: TanStack Queryの `staleTime` を5秒に設定（手動リフレッシュのみ）

### 3.2 互換性要件

#### 既存CLIとの共存

**コマンド引数の互換性**:
```bash
# 既存CLI（Ink UI）- 変更なし
claude-worktree
claude-worktree --help
claude-worktree --version

# Web UI（新規追加）
claude-worktree --web
claude-worktree --web --port 3000
claude-worktree --web --host 0.0.0.0  # GitHub Codespaces対応
```

**設定ファイルの共有**:
```typescript
// ~/.config/claude-worktree/config.json
{
  "defaultBaseBranch": "main",
  "lastUsedTool": "claude-code",
  "webUI": {
    "defaultPort": 3000,
    "defaultHost": "localhost"
  }
}
```

**セッション共有**:
```typescript
// ~/.config/claude-worktree/sessions.json
{
  "lastWorktreePath": "/path/to/.worktrees/feature-web-ui",
  "lastBranch": "feature/web-ui",
  "lastUsedTool": "claude-code",
  "timestamp": 1234567890,
  "repositoryRoot": "/path/to/repo"
}
```

**Web UIからCLIへの切り替え**:
- Web UIで選択した状態（selectedBranch, selectedTool）をセッションに保存
- CLI起動時に最後のセッションを復元

#### 既存コア機能の再利用

**再利用可能な関数**:
```typescript
// src/git.ts - すべての関数をREST API経由で公開
export async function getAllBranches(): Promise<BranchInfo[]>
export async function createBranch(name: string, base: string): Promise<void>
export async function deleteBranch(name: string, force?: boolean): Promise<void>
export async function fetchAllRemotes(options?: { cwd?: string }): Promise<void>

// src/worktree.ts - すべての関数をREST API経由で公開
export async function createWorktree(config: WorktreeConfig): Promise<void>
export async function removeWorktree(path: string, force?: boolean): Promise<void>
export async function getMergedPRWorktrees(): Promise<CleanupTarget[]>

// src/services/WorktreeOrchestrator.ts - そのまま再利用
export class WorktreeOrchestrator {
  async ensureWorktree(branch: string, repoRoot: string, options: EnsureWorktreeOptions): Promise<string>
}
```

**REST API設計**:
```typescript
// web/server/routes/git.ts
import { getAllBranches, createBranch, deleteBranch, fetchAllRemotes } from '../../../src/git.js';

app.get('/api/branches', async (req, res) => {
  try {
    const branches = await getAllBranches();
    res.json(branches);
  } catch (error) {
    res.status(500).json({ error: (error as Error).message });
  }
});

app.post('/api/branches', async (req, res) => {
  const { name, base } = req.body;
  try {
    await createBranch(name, base);
    res.json({ success: true });
  } catch (error) {
    res.status(500).json({ error: (error as Error).message });
  }
});

app.delete('/api/branches/:name', async (req, res) => {
  const { name } = req.params;
  const { force } = req.query;
  try {
    await deleteBranch(name, force === 'true');
    res.json({ success: true });
  } catch (error) {
    res.status(500).json({ error: (error as Error).message });
  }
});
```

**PTYセッション管理**:
```typescript
// web/server/pty/manager.ts
import * as pty from 'node-pty';
import { launchClaudeCode } from '../../../src/claude.js';
import { launchCodexCLI } from '../../../src/codex.js';

interface PTYSession {
  id: string;
  ptyProcess: pty.IPty;
  worktreePath: string;
  tool: 'claude-code' | 'codex-cli';
  createdAt: Date;
  lastActivityAt: Date;
}

const sessions = new Map<string, PTYSession>();

export async function createPTYSession(
  tool: 'claude-code' | 'codex-cli',
  worktreePath: string,
  options: any
): Promise<string> {
  const sessionId = crypto.randomUUID();
  
  // PTYプロセスを作成（AI Toolコマンドを実行）
  const command = tool === 'claude-code' ? 'claude' : 'bunx';
  const args = tool === 'claude-code' 
    ? ['--dangerously-skip-permissions'] 
    : ['@openai/codex@latest', '--yolo'];
  
  const ptyProcess = pty.spawn(command, args, {
    name: 'xterm-256color',
    cols: 80,
    rows: 24,
    cwd: worktreePath,
    env: process.env as any,
  });
  
  sessions.set(sessionId, {
    id: sessionId,
    ptyProcess,
    worktreePath,
    tool,
    createdAt: new Date(),
    lastActivityAt: new Date(),
  });
  
  // 1時間アイドルでセッション削除
  setTimeout(() => {
    const session = sessions.get(sessionId);
    if (session && Date.now() - session.lastActivityAt.getTime() > 3600000) {
      session.ptyProcess.kill();
      sessions.delete(sessionId);
    }
  }, 3600000);
  
  return sessionId;
}

export function getPTYSession(sessionId: string): PTYSession | undefined {
  return sessions.get(sessionId);
}

export function removePTYSession(sessionId: string): void {
  const session = sessions.get(sessionId);
  if (session) {
    session.ptyProcess.kill();
    sessions.delete(sessionId);
  }
}
```

### 3.3 依存関係の追加

**新規依存関係（Web UIのみ）**:
```json
{
  "dependencies": {
    "fastify": "^5.2.0",
    "@fastify/websocket": "^11.1.0",
    "@fastify/cors": "^10.1.0",
    "node-pty": "^1.1.0",
    "@tanstack/react-query": "^5.66.1",
    "zustand": "^5.0.3",
    "xterm": "^5.5.0",
    "xterm-addon-fit": "^0.10.0"
  },
  "devDependencies": {
    "vite": "^6.0.11",
    "@vitejs/plugin-react": "^4.3.4",
    "tailwindcss": "^4.1.7",
    "autoprefixer": "^10.4.20",
    "postcss": "^8.4.49"
  }
}
```

**既存依存関係との互換性**:
- React 19.2.0 - 既存（Ink UI）と共通
- TypeScript 5.8.x - 既存と共通
- Bun 1.0+ - 既存と共通
- Vitest 4.0.8 - Web UIのテストにも使用

**ビルド設定の追加**:
```json
{
  "scripts": {
    "build": "tsc && vite build",
    "build:cli": "tsc",
    "build:web": "vite build",
    "dev:cli": "tsc --watch",
    "dev:web": "vite",
    "start:cli": "bun ./dist/index.js",
    "start:web": "bun ./dist/index.js --web"
  }
}
```

## 4. 推奨事項

### 4.1 実装アプローチ

#### Phase 0: 調査・設計（完了）
- [x] 既存コードベース分析
- [x] 技術スタック決定
- [x] REST API設計
- [x] PTYセッション管理設計
- [x] WebSocketプロトコル設計

#### Phase 1: 最小限のPOC（目標: 2-3日）
**目的**: PTY + WebSocket + xterm.jsの技術検証

**スコープ**:
1. Fastifyサーバーの基本セットアップ
   - `web/server/index.ts` - エントリーポイント
   - `web/server/routes/health.ts` - ヘルスチェックAPI
2. PTYセッション作成API
   - `POST /api/pty/create` - Claude Code起動
   - `GET /pty/:sessionId` - WebSocket接続
3. 最小限のReactフロントエンド
   - `web/client/src/main.tsx` - エントリーポイント
   - `web/client/src/components/Terminal.tsx` - xterm.jsラッパー

**成果物**:
- `bunx claude-worktree --web` でブラウザが開き、Claude Codeのターミナルが表示される
- キーボード入力・ANSI色付けが正常に動作

**検証項目**:
- [ ] PTYプロセスが正常に起動するか
- [ ] WebSocketでリアルタイム通信できるか
- [ ] xterm.jsでANSI色付けが正しく表示されるか
- [ ] キーボード入力（特殊キー含む）が正常に動作するか
- [ ] プロセス終了時にリソースがクリーンアップされるか

#### Phase 2: Git/Worktree REST API（目標: 3-4日）
**目的**: 既存のコア機能をREST API経由で公開

**スコープ**:
1. Git操作API
   - `GET /api/branches` - ブランチ一覧取得
   - `POST /api/branches` - ブランチ作成
   - `DELETE /api/branches/:name` - ブランチ削除
   - `POST /api/git/fetch` - リモート更新
   - `POST /api/git/pull` - Fast-forward pull
   - `GET /api/git/divergence` - 差分確認
2. Worktree操作API
   - `GET /api/worktrees` - Worktree一覧取得
   - `POST /api/worktrees` - Worktree作成
   - `DELETE /api/worktrees/:path` - Worktree削除
   - `GET /api/worktrees/cleanup` - クリーンアップ候補取得
3. エラーハンドリング
   - GitError / WorktreeErrorをJSON形式で返却
   - リトライ可能エラーの判定

**成果物**:
- Postman / cURLでAPIを叩いて、Git/Worktree操作ができる
- エラー時に適切なHTTPステータスコードとエラーメッセージが返る

**検証項目**:
- [ ] 並列実行時のGitロック競合が発生しないか
- [ ] エラー時にセッション状態が残らないか
- [ ] 1000+ ブランチでもレスポンスが1秒以内か

#### Phase 3: フルUIの実装（目標: 5-7日）
**目的**: Ink UIと同等の機能をWeb UIで実装

**スコープ**:
1. ブランチ一覧画面
   - TanStack Queryでブランチ一覧をフェッチ
   - react-windowで仮想スクロール
   - 検索・フィルタリング機能
2. AI Tool選択画面
   - Claude Code / Codex CLI / カスタムツール
   - 実行モード選択（normal / continue / resume）
   - skip permissions フラグ
3. ワークフロー統合
   - Worktree準備 → PTYセッション作成 → Terminal表示
   - エラー時のリトライ・ロールバック
4. リアルタイムフィードバック
   - Git操作の進捗表示（スピナー）
   - クリーンアップのバッチ処理UI

**成果物**:
- Ink UIと同等の機能がWeb UIで動作
- ブラウザでブランチ選択 → AI Tool起動 → ターミナル操作が完結

**検証項目**:
- [ ] Ink UIと同じワークフローが再現できるか
- [ ] リアルタイムフィードバックがスムーズか
- [ ] エラー時のロールバックが正常に動作するか

#### Phase 4: 追加機能・最適化（目標: 3-5日）
**目的**: Web UI独自の機能とパフォーマンス最適化

**スコープ**:
1. セッション管理
   - アクティブなPTYセッション一覧表示
   - セッションの切り替え（タブ機能）
   - セッションの保存・復元
2. GitHub PR統合
   - マージ済みPRのクリーンアップUI
   - PRステータスのバッジ表示
3. ターミナルの高度な機能
   - リサイズ対応（xterm-addon-fit）
   - コピー&ペースト
   - 検索機能（xterm-addon-search）
4. パフォーマンス最適化
   - WebSocketメッセージのバッチ処理
   - TanStack Queryのキャッシュ戦略
   - React.memoによる再レンダリング削減

**成果物**:
- 複数のPTYセッションをタブで切り替えられる
- GitHub PR統合が正常に動作
- 1000+ ブランチでもスムーズに動作

**検証項目**:
- [ ] 10個のPTYセッションを同時起動しても動作するか
- [ ] ターミナルのリサイズが正常に動作するか
- [ ] メモリリークが発生しないか

#### Phase 5: テスト・ドキュメント（目標: 2-3日）
**目的**: 本番環境での品質担保

**スコープ**:
1. ユニットテスト
   - PTYセッション管理のテスト
   - REST APIのテスト
   - Reactコンポーネントのテスト
2. E2Eテスト
   - Playwright でブラウザ自動テスト
   - ブランチ選択 → AI Tool起動の全フロー
3. ドキュメント
   - README.mdにWeb UIの説明を追加
   - API仕様書（OpenAPI）
   - アーキテクチャ図（Mermaid）

**成果物**:
- テストカバレッジ: 80%以上
- E2Eテストで全ワークフローをカバー
- README.mdにクイックスタートガイド

**検証項目**:
- [ ] CI/CDでテストが自動実行されるか
- [ ] ドキュメントが最新の実装と一致しているか

### 4.2 リスク軽減策

#### リスク1: PTYプロセスのゾンビ化

**問題**: WebSocket切断時にPTYプロセスが残り続ける

**対策**:
```typescript
// web/server/pty/manager.ts
export function setupWebSocket(ws: WebSocket, sessionId: string) {
  const session = getPTYSession(sessionId);
  if (!session) {
    ws.close();
    return;
  }
  
  // WebSocket切断時にPTYプロセスをkill
  ws.on('close', () => {
    session.ptyProcess.kill();
    sessions.delete(sessionId);
  });
  
  // PTYプロセス終了時にWebSocketをクローズ
  session.ptyProcess.onExit(() => {
    ws.close();
    sessions.delete(sessionId);
  });
}
```

**検証**:
- ブラウザを強制終了してもPTYプロセスが残らないことを確認
- `ps aux | grep claude` でゾンビプロセスをチェック

#### リスク2: Gitロック競合

**問題**: 複数のWorktreeで同時に `git fetch` すると、`.git/index.lock` で競合

**対策**:
```typescript
// web/server/utils/git-queue.ts
import PQueue from 'p-queue';

const gitQueue = new PQueue({ concurrency: 1 });

export async function queuedGitOperation<T>(fn: () => Promise<T>): Promise<T> {
  return gitQueue.add(fn);
}

// 使用例
import { queuedGitOperation } from './utils/git-queue.js';
import { fetchAllRemotes } from '../../../src/git.js';

app.post('/api/git/fetch', async (req, res) => {
  try {
    await queuedGitOperation(() => fetchAllRemotes({ cwd: req.body.repoRoot }));
    res.json({ success: true });
  } catch (error) {
    res.status(500).json({ error: (error as Error).message });
  }
});
```

**検証**:
- 10個のWorktreeで同時に `git fetch` してもエラーが発生しないことを確認
- ロック待機のログを出力して、キューイングが正常に動作していることを確認

#### リスク3: WebSocketメッセージのバッファオーバーフロー

**問題**: 大量のPTY出力（`git log --all --oneline` など）でWebSocketがバッファオーバーフロー

**対策**:
```typescript
// web/server/pty/manager.ts
export function setupWebSocket(ws: WebSocket, sessionId: string) {
  const session = getPTYSession(sessionId);
  if (!session) {
    ws.close();
    return;
  }
  
  // PTY出力を1KBチャンクに分割して送信
  let buffer = '';
  session.ptyProcess.onData((data) => {
    buffer += data;
    while (buffer.length > 1024) {
      const chunk = buffer.slice(0, 1024);
      buffer = buffer.slice(1024);
      ws.send(JSON.stringify({ type: 'output', data: chunk }));
    }
    if (buffer.length > 0) {
      ws.send(JSON.stringify({ type: 'output', data: buffer }));
      buffer = '';
    }
  });
}
```

**検証**:
- `git log --all --oneline` で数万行の出力をテスト
- WebSocketのメッセージサイズが1KB以下に制限されていることを確認

#### リスク4: メモリリーク

**問題**: React コンポーネントのアンマウント時にイベントリスナーが残る

**対策**:
```typescript
// web/client/src/components/Terminal.tsx
import { useEffect, useRef } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';

export function TerminalComponent({ sessionId }: { sessionId: string }) {
  const terminalRef = useRef<HTMLDivElement>(null);
  const xtermRef = useRef<Terminal | null>(null);
  const wsRef = useRef<WebSocket | null>(null);
  
  useEffect(() => {
    if (!terminalRef.current) return;
    
    const terminal = new Terminal();
    const fitAddon = new FitAddon();
    terminal.loadAddon(fitAddon);
    terminal.open(terminalRef.current);
    fitAddon.fit();
    xtermRef.current = terminal;
    
    const ws = new WebSocket(`ws://localhost:3000/pty/${sessionId}`);
    wsRef.current = ws;
    
    ws.onmessage = (event) => {
      const data = JSON.parse(event.data);
      if (data.type === 'output') {
        terminal.write(data.data);
      }
    };
    
    terminal.onData((data) => {
      ws.send(JSON.stringify({ type: 'input', data }));
    });
    
    // クリーンアップ
    return () => {
      terminal.dispose();
      ws.close();
    };
  }, [sessionId]);
  
  return <div ref={terminalRef} />;
}
```

**検証**:
- React DevToolsで不要なイベントリスナーが残っていないことを確認
- Chrome DevToolsの Memory Profiler でメモリリークをチェック

#### リスク5: TypeScript型の不一致

**問題**: REST API レスポンスの型がフロントエンドと一致しない

**対策**:
```typescript
// 共通の型定義を作成（src/types/ に配置）
// src/types/api.ts
export interface BranchResponse {
  name: string;
  type: 'local' | 'remote';
  branchType: 'main' | 'develop' | 'feature' | 'hotfix' | 'release' | 'other';
  isCurrent: boolean;
  latestCommitTimestamp?: number;
}

export interface CreateBranchRequest {
  name: string;
  base: string;
}

// サーバー側で使用
// web/server/routes/git.ts
import type { BranchResponse, CreateBranchRequest } from '../../../src/types/api.js';

app.get('/api/branches', async (req, res) => {
  const branches: BranchResponse[] = await getAllBranches();
  res.json(branches);
});

app.post('/api/branches', async (req, res) => {
  const { name, base }: CreateBranchRequest = req.body;
  await createBranch(name, base);
  res.json({ success: true });
});

// クライアント側で使用
// web/client/src/api/branches.ts
import type { BranchResponse, CreateBranchRequest } from '../../../src/types/api.js';

export async function fetchBranches(): Promise<BranchResponse[]> {
  const res = await fetch('/api/branches');
  return res.json();
}

export async function createBranch(data: CreateBranchRequest): Promise<void> {
  await fetch('/api/branches', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(data),
  });
}
```

**検証**:
- TypeScript コンパイル時にエラーが発生しないことを確認
- API リクエスト/レスポンスの型が正しく推論されることを確認

### 4.3 次のステップ

#### 1. Phase 1の実装開始
- `web/server/index.ts` - Fastifyサーバーのセットアップ
- `web/server/pty/manager.ts` - PTYセッション管理
- `web/client/src/main.tsx` - Reactフロントエンドのエントリーポイント
- `web/client/src/components/Terminal.tsx` - xterm.jsラッパー

#### 2. POC デモの作成
- `bunx claude-worktree --web` でブラウザが開く
- Claude Codeのターミナルが表示される
- キーボード入力が正常に動作する

#### 3. フィードバック収集
- 実際に使ってみて、UXの問題点を洗い出す
- PTYセッションの安定性を確認
- パフォーマンスボトルネックを特定

#### 4. Phase 2以降の計画見直し
- POCで得られた知見を元に、実装スコープを調整
- 優先度の高い機能から順に実装

## 5. 参考資料

### 5.1 既存プロジェクトの調査

**VS Code Server (code-server)**:
- GitHub: https://github.com/coder/code-server
- PTY実装: node-ptyを使用
- WebSocket: ws を使用
- ターミナル: xterm.jsを使用

**Gotty**:
- GitHub: https://github.com/yudai/gotty
- Go言語実装（参考程度）
- WebSocket経由でターミナルを公開

**ttyd**:
- GitHub: https://github.com/tsl0922/ttyd
- C言語実装（参考程度）
- libwebsockets + libuvでWebSocketターミナル

**GitHub Codespaces**:
- 公式ドキュメント: https://docs.github.com/en/codespaces
- VS Code Server + Dev Containers
- ポートフォワーディング（WebSocket対応）

### 5.2 技術ドキュメント

**Fastify**:
- 公式ドキュメント: https://fastify.dev/
- WebSocketプラグイン: https://github.com/fastify/fastify-websocket

**node-pty**:
- GitHub: https://github.com/microsoft/node-pty
- Windows ConPTY: https://devblogs.microsoft.com/commandline/windows-command-line-introducing-the-windows-pseudo-console-conpty/

**xterm.js**:
- 公式ドキュメント: https://xtermjs.org/
- API Reference: https://github.com/xtermjs/xterm.js/blob/master/typings/xterm.d.ts

**TanStack Query**:
- 公式ドキュメント: https://tanstack.com/query/latest
- React Query Tutorial: https://tkdodo.eu/blog/practical-react-query

**Zustand**:
- 公式ドキュメント: https://github.com/pmndrs/zustand

### 5.3 関連技術の比較

**WebSocketプロトコル vs Server-Sent Events (SSE)**:
- WebSocket: 双方向通信（クライアント → サーバー、サーバー → クライアント）
- SSE: 単方向通信（サーバー → クライアントのみ）
- 選択理由: PTYは双方向通信が必須（キーボード入力を送信する必要がある）

**REST API vs GraphQL**:
- REST: シンプル、キャッシュしやすい
- GraphQL: 柔軟なクエリ、1リクエストで複数リソース取得
- 選択理由: REST APIで十分（複雑なクエリは不要）

**React vs Vue.js vs Svelte**:
- React: 既存のInk UIと同じ、エコシステムが大きい
- Vue.js: 学習曲線が緩やか
- Svelte: コンパイル時最適化
- 選択理由: Reactで既存コードベースとの一貫性を保つ
