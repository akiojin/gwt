# クイックスタートガイド: ヘッダーへの起動ディレクトリ表示

**SPEC ID**: SPEC-e07f3844
**対象**: 開発者
**最終更新**: 2025-01-05

## 概要

このガイドでは、claude-worktreeのヘッダー部分に起動ディレクトリを表示する機能の実装手順を説明します。3つのファイルを変更し、propsベースでデータを渡すシンプルな実装です。

## 前提条件

- Node.js 18+ または Bun 1.0+ がインストール済み
- TypeScript 5.x の基本知識
- React と React Ink の基本的な理解
- プロジェクトのビルドが成功する状態

## セットアップ

### 1. 依存関係のインストール

```bash
bun install
```

### 2. ビルドの確認

```bash
bun run build
```

エラーがないことを確認してから実装を開始してください。

## 実装手順

### ステップ1: Header.tsxの拡張

**ファイル**: `src/ui/components/parts/Header.tsx`

#### 1.1 HeaderPropsインターフェースの更新

`version`プロパティの直後に`workingDirectory`を追加します：

```typescript
export interface HeaderProps {
  title: string;
  titleColor?: string;
  dividerChar?: string;
  showDivider?: boolean;
  width?: number;
  version?: string | null | undefined;
  /**
   * 起動時の作業ディレクトリの絶対パス
   * - string: ディレクトリパスが利用可能
   * - undefined: ディレクトリ情報未提供
   * @default undefined
   */
  workingDirectory?: string;
}
```

#### 1.2 Header関数コンポーネントの更新

propsの分割代入に`workingDirectory`を追加：

```typescript
export const Header = React.memo(function Header({
  title,
  titleColor = 'cyan',
  dividerChar = '─',
  showDivider = true,
  width = 80,
  version,
  workingDirectory,  // ← 追加
}: HeaderProps) {
```

#### 1.3 レンダリングロジックの更新

dividerの直後に`workingDirectory`の表示を追加：

```typescript
return (
  <Box flexDirection="column">
    <Box>
      <Text bold color={titleColor}>
        {displayTitle}
      </Text>
    </Box>
    {showDivider && (
      <Box>
        <Text dimColor>{divider}</Text>
      </Box>
    )}
    {/* 追加: Working Directory表示 */}
    {workingDirectory && (
      <Box>
        <Text dimColor>Working Directory: </Text>
        <Text>{workingDirectory}</Text>
      </Box>
    )}
  </Box>
);
```

**ポイント**:
- `{workingDirectory && ...}`で条件付きレンダリング
- `<Text dimColor>`でラベルを薄く表示
- `<Text>`で実際のパスを通常の明るさで表示

### ステップ2: BranchListScreen.tsxの更新

**ファイル**: `src/ui/components/screens/BranchListScreen.tsx`

#### 2.1 BranchListScreenPropsインターフェースの更新

`version`プロパティの直後に`workingDirectory`を追加：

```typescript
export interface BranchListScreenProps {
  branches: BranchItem[];
  stats: Statistics;
  onSelect: (branch: BranchItem) => void;
  onNavigate?: (screen: string) => void;
  onQuit?: () => void;
  onCleanupCommand?: () => void;
  onRefresh?: () => void;
  loading?: boolean;
  error?: Error | null;
  lastUpdated?: Date | null;
  loadingIndicatorDelay?: number;
  cleanupUI?: CleanupUIState;
  version?: string | null;
  workingDirectory?: string;  // ← 追加
}
```

#### 2.2 BranchListScreen関数コンポーネントの更新

propsの分割代入に`workingDirectory`を追加：

```typescript
export function BranchListScreen({
  branches,
  stats,
  onSelect,
  onNavigate,
  onQuit,
  onCleanupCommand,
  onRefresh,
  loading = false,
  error = null,
  lastUpdated = null,
  loadingIndicatorDelay,
  cleanupUI,
  version,
  workingDirectory,  // ← 追加
}: BranchListScreenProps) {
```

#### 2.3 Headerコンポーネント呼び出しの更新

121行目付近のHeader呼び出しを更新：

```typescript
<Header
  title="Claude Worktree - Branch Selection"
  titleColor="cyan"
  version={version}
  workingDirectory={workingDirectory}  // ← 追加
/>
```

### ステップ3: App.tsxの更新

**ファイル**: `src/ui/components/App.tsx`

#### 3.1 起動ディレクトリの取得

App関数コンポーネントの先頭（65行目付近、useAppの直後）で`workingDirectory`を取得：

```typescript
export function App({ onExit, loadingIndicatorDelay = 300 }: AppProps) {
  const { exit } = useApp();

  // 起動ディレクトリの取得
  const workingDirectory = process.cwd();

  const { branches, worktrees, loading, error, refresh, lastUpdated } = useGitData({
    enableAutoRefresh: false,
  });
  // ...
}
```

**ポイント**:
- `process.cwd()`は同期的で高速
- シンボリックリンク解決済みの絶対パスを返す
- useStateは不要（起動時に一度だけ取得）

#### 3.2 BranchListScreenへのprops追加

BranchListScreenコンポーネントに`workingDirectory`を渡す箇所を探し、追加：

```typescript
<BranchListScreen
  branches={branchItems}
  stats={stats}
  onSelect={handleBranchSelect}
  onNavigate={navigateTo}
  onQuit={handleQuit}
  onCleanupCommand={handleCleanupCommand}
  onRefresh={refresh}
  loading={loading}
  error={error}
  lastUpdated={lastUpdated}
  loadingIndicatorDelay={loadingIndicatorDelay}
  cleanupUI={cleanupUIState}
  version={version}
  workingDirectory={workingDirectory}  // ← 追加
/>
```

## 検証手順

### 1. ビルドテスト

```bash
bun run build
```

TypeScriptエラーがないことを確認します。

### 2. 手動テスト

#### テスト1: 基本動作確認

```bash
cd /home/user/project-a
bunx .
```

期待される表示:
```
Claude Worktree - Branch Selection v1.17.0
────────────────────────────────────────────────────────────────────────────────
Working Directory: /home/user/project-a
Local: 13  Remote: 78  Worktrees: 7  Changes: 0  Updated: 0s ago
```

#### テスト2: 異なるディレクトリ

```bash
cd /var/www/project-b
bunx .
```

`Working Directory: /var/www/project-b`が表示されることを確認。

#### テスト3: 長いパス

```bash
cd /home/user/development/projects/client-name/application/backend
bunx .
```

パスが折り返されて完全に表示されることを確認。

#### テスト4: シンボリックリンク

```bash
ln -s /home/user/project-a /tmp/link-to-project
cd /tmp/link-to-project
bunx .
```

実際のパス（`/home/user/project-a`）が表示されることを確認。

## トラブルシューティング

### TypeScriptエラー: Property 'workingDirectory' does not exist

**原因**: propsの型定義が不足

**解決策**:
1. `HeaderProps`に`workingDirectory?: string`を追加
2. `BranchListScreenProps`に`workingDirectory?: string`を追加

### ディレクトリが表示されない

**原因**: propsのリレーが不完全

**解決策**:
1. App.tsx: `const workingDirectory = process.cwd();`を確認
2. App.tsx → BranchListScreen: props渡しを確認
3. BranchListScreen → Header: props渡しを確認
4. Header.tsx: 条件付きレンダリング `{workingDirectory && ...}` を確認

### ビルドは成功するが実行時エラー

**原因**: React Inkのバージョン不一致

**解決策**:
```bash
bun install
bun run build
```

## よくある質問

### Q: なぜuseStateを使わないのですか？

**A**: `process.cwd()`は起動時に一度だけ取得すれば十分で、変更されることはありません。useStateは状態が変化する場合に使用します。

### Q: getRepositoryRoot()ではダメですか？

**A**: `getRepositoryRoot()`はリポジトリのルートを返しますが、仕様では「起動ディレクトリ」を表示することが要件です。また、非同期処理が不要にシンプルさを保つため`process.cwd()`を使用します。

### Q: Working Directory:を日本語にできますか？

**A**: 可能ですが、CLIツールの標準では英語ラベルが一般的です。変更する場合はHeader.tsxの`<Text dimColor>Working Directory: </Text>`部分を変更してください。

### Q: パスが長すぎて見づらい場合は？

**A**: React Inkは自動で折り返しますが、将来的にホームディレクトリを`~`で短縮する拡張が考えられます。現在の仕様では完全な絶対パス表示が要件です。

## 次のステップ

1. ✅ 実装完了
2. ⏭️ `/speckit.tasks`でタスク分解を確認
3. ⏭️ `/speckit.implement`で自動実装を試す

## 参考資料

- [React Ink ドキュメント](https://github.com/vadimdemedes/ink)
- [Node.js process.cwd()](https://nodejs.org/api/process.html#processcwd)
- [TypeScript Handbook](https://www.typescriptlang.org/docs/handbook/intro.html)
- [研究レポート](./research.md)
- [実装計画](./plan.md)
