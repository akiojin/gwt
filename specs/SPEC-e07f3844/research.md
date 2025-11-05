# 調査レポート: ヘッダーへの起動ディレクトリ表示

**SPEC ID**: SPEC-e07f3844
**調査日**: 2025-01-05
**調査者**: Claude (AI Agent)

## 概要

claude-worktreeのヘッダー部分に起動ディレクトリの絶対パスを表示する機能の実装に向けた技術調査を実施しました。既存のコードベースを分析し、最適な実装方法を決定しました。

## 1. 既存のコードベース分析

### 1.1 Header.tsxの現在の構造

**ファイルパス**: `src/ui/components/parts/Header.tsx`

**主要な発見**:
- React.memoで最適化されたReact Inkコンポーネント
- HeaderPropsインターフェースで型安全性を確保
- 既存のprops:
  - `title: string` - ヘッダータイトル
  - `titleColor?: string` - タイトルの色（デフォルト: 'cyan'）
  - `dividerChar?: string` - 区切り文字（デフォルト: '─'）
  - `showDivider?: boolean` - 区切り線の表示/非表示
  - `width?: number` - 区切り線の幅（デフォルト: 80）
  - `version?: string | null | undefined` - バージョン情報（オプショナル）

**レンダリング構造**:
```tsx
<Box flexDirection="column">
  <Box>{タイトル + バージョン}</Box>
  {showDivider && <Box>{区切り線}</Box>}
</Box>
```

**重要な設計パターン**:
- すべてのpropsはオプショナル（`title`以外）
- バージョン表示は条件付き: `version ? ${title} v${version}` : title`
- flexDirection="column"で垂直レイアウト

### 1.2 BranchListScreen.tsxのHeader呼び出し

**ファイルパス**: `src/ui/components/screens/BranchListScreen.tsx`

**現在の呼び出しパターン** (121行目付近):
```tsx
<Header
  title="Claude Worktree - Branch Selection"
  titleColor="cyan"
  version={version}
/>
```

**propsの渡し方**:
- BranchListScreenPropsとして`version?: string | null`を受け取る
- そのまま`version`としてHeaderに渡す

### 1.3 App.tsxでのデータフロー

**ファイルパス**: `src/ui/components/App.tsx`

**バージョン取得の実装** (89-94行目):
```tsx
useEffect(() => {
  getPackageVersion()
    .then(setVersion)
    .catch(() => setVersion(null));
}, []);
```

**データフローパターン**:
1. App.tsxで`getPackageVersion()`を呼び出し
2. useStateで`version`を管理
3. BranchListScreenに`version`をpropsとして渡す
4. BranchListScreenがHeaderに`version`を渡す

**重要な発見**:
- App.tsxは非同期処理（`getPackageVersion()`）を適切に処理
- エラー時は`null`を設定（Headerは`null`を許容）
- この同じパターンを起動ディレクトリにも適用可能

### 1.4 ディレクトリ取得方法

**オプション1: process.cwd()**
- Node.jsの標準API
- 同期的に現在の作業ディレクトリを取得
- シンボリックリンク解決済みのパスを返す
- **推奨**: シンプルで確実

**オプション2: getRepositoryRoot()** (`src/git.ts` 60-82行目)
- 非同期関数（async/await）
- `git rev-parse --git-common-dir`でGitリポジトリのルートを取得
- worktree対応（メインリポジトリの.gitディレクトリを返す）
- **非推奨**: この機能の目的は「起動ディレクトリ」であり「リポジトリルート」ではない

## 2. 技術的決定事項

### 決定1: ディレクトリ取得方法

**決定**: `process.cwd()`を使用

**根拠**:
1. **仕様との整合性**: 仕様書では「起動ディレクトリ」を表示することが要件
2. **シンプルさ**: 同期的で追加のエラーハンドリングが不要
3. **パフォーマンス**: 即座に値を取得可能
4. **シンボリックリンク対応**: `process.cwd()`は既に解決済みのパスを返す

**実装コード例**:
```tsx
const workingDirectory = process.cwd();
```

**代替案**:
- `getRepositoryRoot()`は非同期で、リポジトリルートを返すため、今回の要件には不適切

### 決定2: 表示位置

**決定**: 区切り線（divider）の直後、統計情報の前

**根拠**:
1. **ユーザー要件**: 「区切り線の下」という明確な指示
2. **視覚的階層**: タイトル → 区切り線 → コンテキスト情報（ディレクトリ） → データ（統計）という自然な流れ
3. **既存パターンとの整合性**: バージョンがタイトルに統合されているように、ディレクトリも独立した行として追加

**実装方法**:
- Headerコンポーネント内でdividerの後に新しい`<Box>`を追加
- `workingDirectory`が提供された場合のみ表示

### 決定3: Props名とインターフェース

**決定**: `workingDirectory?: string`

**根拠**:
1. **明確性**: `workingDirectory`は起動ディレクトリを明確に表現
2. **オプショナル**: 既存コードへの影響を最小化（後方互換性）
3. **型安全性**: TypeScriptの型システムで保護

**代替案の検討**:
- `directory`: 汎用的すぎる
- `currentPath`: パスではなくディレクトリを表現したい
- `cwd`: 略称は可読性が低い

### 決定4: 表示フォーマット

**決定**: `Working Directory: /absolute/path`

**根拠**:
1. **仕様要件**: "Working Directory: "というラベル指定あり
2. **国際化**: 英語ラベルでCLIツールの標準に準拠
3. **視認性**: ラベルとパスが明確に分離

**実装コード例**:
```tsx
{workingDirectory && (
  <Box>
    <Text dimColor>Working Directory: </Text>
    <Text>{workingDirectory}</Text>
  </Box>
)}
```

## 3. React Inkの制約と対応

### 3.1 レイアウトシステム

**制約**:
- React InkはFlexboxベースのレイアウト
- `<Box flexDirection="column">`で垂直スタック
- 各要素は自動で改行

**対応**:
- 既存のHeader構造を維持し、新しい`<Box>`を追加するだけで対応可能
- 折り返しはReact Inkが自動処理

### 3.2 長いパスの処理

**調査結果**:
- React Inkは80文字を超えるテキストを自動で折り返す
- `<Text wrap="truncate">`で切り詰めも可能だが、情報の完全性を優先

**決定**: デフォルトの折り返し動作を使用（カスタム処理なし）

### 3.3 スタイリング

**既存パターン**:
- `<Text dimColor>`でラベルを薄く表示
- `<Text>`でデフォルトの明るさで値を表示

**採用**: 既存のパターンを踏襲

## 4. 制約と依存関係

### 4.1 技術的制約

1. **TypeScript厳格モード**: すべてのpropsは型定義が必須
2. **React.memo最適化**: props変更時のみ再レンダリング
3. **React Ink APIバージョン**: 既存のバージョンとの互換性維持

### 4.2 互換性の確認

**後方互換性**:
- `workingDirectory`はオプショナルなprops
- 既存のHeaderの使用箇所に影響なし
- BranchListScreen以外のHeaderの使用箇所（もしあれば）も動作継続

**将来の拡張性**:
- 他のコンテキスト情報（Git branch、user情報など）も同様のパターンで追加可能
- Headerコンポーネントの責務は変わらず「情報表示」に留まる

## 5. ベストプラクティス

### 5.1 既存コードパターンの活用

**確認された良いパターン**:
1. **オプショナルなprops**: 機能追加時も既存コードに影響しない
2. **条件付きレンダリング**: `{condition && <Component />}`で表示制御
3. **型安全性**: TypeScriptインターフェースで厳密な型定義
4. **React.memo**: 不要な再レンダリングを防止

### 5.2 CLIツールの標準

**参照した標準**:
- Docker CLIの`Working Directory:`表示
- Git CLIの`pwd`出力フォーマット
- VSCodeの統合ターミナルのヘッダー表示

## 6. リスク評価

### 6.1 技術的リスク

| リスク | 深刻度 | 発生確率 | 緩和策 |
|--------|--------|----------|--------|
| 長いパスでの表示崩れ | 低 | 中 | React Inkの自動折り返しで対応 |
| propsの型エラー | 低 | 低 | TypeScriptの型チェックで防止 |
| パフォーマンス影響 | 極低 | 極低 | `process.cwd()`は同期で高速 |

### 6.2 ユーザー体験リスク

| リスク | 深刻度 | 発生確率 | 緩和策 |
|--------|--------|----------|--------|
| 情報過多 | 低 | 低 | `dimColor`でラベルを目立たなくする |
| 視認性の低下 | 低 | 低 | 既存の表示パターンを踏襲 |

## 7. 実装推奨事項

### 7.1 変更ファイル

1. **src/ui/components/parts/Header.tsx**
   - `HeaderProps`に`workingDirectory?: string`を追加
   - レンダリングロジックに`workingDirectory`表示を追加

2. **src/ui/components/screens/BranchListScreen.tsx**
   - `BranchListScreenProps`に`workingDirectory?: string`を追加
   - Headerコンポーネントに`workingDirectory`を渡す

3. **src/ui/components/App.tsx**
   - `const workingDirectory = process.cwd();`を追加
   - BranchListScreenに`workingDirectory`を渡す

### 7.2 実装順序

1. Header.tsxのprops追加（ビルド確認）
2. App.tsxでの`process.cwd()`取得
3. propsのリレー（App → BranchListScreen → Header）
4. 表示確認とスタイル調整

### 7.3 テスト計画

**ビルドテスト**:
```bash
bun run build
```

**手動テスト**:
1. `/home/user/project-a`から起動 → 表示確認
2. `/var/www/project-b`から起動 → 表示確認
3. 長いパス（100文字超）で起動 → 折り返し確認
4. シンボリックリンク経由で起動 → 実パス表示確認

## 8. まとめ

### 最終決定事項

| 項目 | 決定内容 |
|------|----------|
| ディレクトリ取得方法 | `process.cwd()` |
| 表示位置 | 区切り線の直後 |
| Props名 | `workingDirectory?: string` |
| 表示フォーマット | `Working Directory: /absolute/path` |
| スタイリング | `dimColor`ラベル + 通常テキスト値 |

### 次のステップ

1. ✅ 調査完了
2. ⏭️ Phase 1: quickstart.mdの作成（実装ガイド）
3. ⏭️ /speckit.tasksでタスク分解
4. ⏭️ /speckit.implementで実装実行

## 参考資料

- [React Ink ドキュメント](https://github.com/vadimdemedes/ink)
- [Node.js process.cwd()](https://nodejs.org/api/process.html#processcwd)
- [TypeScript Handbook - Interfaces](https://www.typescriptlang.org/docs/handbook/interfaces.html)
- 既存のclaude-worktreeコードベース
