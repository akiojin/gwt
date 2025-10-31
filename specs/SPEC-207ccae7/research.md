# 技術調査: アプリケーションバージョン表示機能

**仕様ID**: `SPEC-207ccae7` | **日付**: 2025-10-31
**目的**: 既存のコードベースパターンを理解し、バージョン表示機能の実装方法を決定する

## 1. 既存のコードベース分析

### 1.1 getPackageVersion()関数の実装

**場所**: `src/utils.ts:48-60`

```typescript
export async function getPackageVersion(): Promise<string | null> {
  try {
    const currentDir = getCurrentDirname();
    const packageJsonPath = path.resolve(currentDir, "..", "package.json");

    const packageJsonContent = await readFile(packageJsonPath, "utf-8");
    const packageJson: PackageJson = JSON.parse(packageJsonContent);

    return packageJson.version || null;
  } catch {
    return null;
  }
}
```

**特徴**:
- 非同期関数（`async/await`）
- `getCurrentDirname()`を使用してカレントディレクトリを取得
- package.jsonは`src/`の1つ上のディレクトリから取得
- エラー時は`null`を返す（適切なエラーハンドリング）
- 既にTypeScript型定義済み（`PackageJson`インターフェース）

**結論**: この関数をそのまま再利用可能。新規実装は不要。

### 1.2 Header.tsxコンポーネントの構造

**場所**: `src/ui/components/parts/Header.tsx`

**現在のインターフェース**:
```typescript
export interface HeaderProps {
  title: string;
  titleColor?: string;
  dividerChar?: string;
  showDivider?: boolean;
  width?: number;
}
```

**実装の特徴**:
- `React.memo`で最適化されている（不要な再レンダリングを防止）
- Ink.jsの`<Box>`と`<Text>`コンポーネントを使用
- タイトルとオプショナルな区切り線を表示
- デフォルト値: `titleColor='cyan'`, `dividerChar='─'`, `showDivider=true`, `width=80`

**使用例**（BranchListScreen.tsx:119）:
```tsx
<Header title="Claude Worktree - Branch Selection" titleColor="cyan" />
```

**拡張方法の決定**:
- `version?: string | null`をオプショナルプロップとして追加
- `version`が提供された場合、タイトルの後に表示: `"${title} v${version}"`
- `version`が`null`または未提供の場合、タイトルのみ表示（既存の動作維持）
- `React.memo`の比較関数は変更不要（shallowCompareで十分）

### 1.3 index.tsのCLI引数パース処理

**場所**: `src/index.ts:215-224`

**現在の実装**:
```typescript
export async function main(): Promise<void> {
  try {
    // Parse command line arguments
    const args = process.argv.slice(2);
    const showHelpFlag = args.includes("-h") || args.includes("--help");

    if (showHelpFlag) {
      showHelp();
      return;
    }
    // ... rest of the code
  }
}
```

**`showHelp()`関数**（index.ts:29-42）:
```typescript
function showHelp(): void {
  console.log(`
Worktree Manager

Usage: claude-worktree [options]

Options:
  -h, --help      Show this help message

Description:
  Interactive Git worktree manager with AI tool selection...
`);
}
```

**拡張方法の決定**:
- `showHelp()`と同様のパターンで`showVersion()`関数を追加
- `--version`と`-v`フラグを検出
- `showVersion()`関数内で`getPackageVersion()`を呼び出し
- バージョン取得成功時: バージョンを表示して`return`（早期終了）
- バージョン取得失敗時: エラーメッセージを表示して`return`
- `showHelp()`の前に`showVersion()`チェックを配置（優先度順）

### 1.4 画面コンポーネントでのHeaderの使用方法

**調査結果**:
- 全7画面でHeaderコンポーネントを使用:
  - BranchListScreen
  - BranchCreatorScreen
  - WorktreeManagerScreen
  - SessionSelectorScreen
  - PRCleanupScreen
  - ExecutionModeSelectorScreen
  - AIToolSelectorScreen

**使用パターン**:
```tsx
<Header title="Claude Worktree - [Screen Name]" titleColor="cyan" />
```

**修正方針**:
- 各画面コンポーネントで`getPackageVersion()`を呼び出す必要があるか？
  - **No**: App.tsxレベルで一度だけ取得し、propsとして各画面に渡す方が効率的
  - ただし、仕様では「各画面でバージョンを表示」とあるため、各画面で個別に取得も可能

**最終決定**:
- **アプローチ1**: App.tsx で `useEffect` を使用してバージョンを一度取得し、各画面にpropsで渡す（推奨）
- **アプローチ2**: 各画面で個別に `getPackageVersion()` を呼び出す（シンプルだが非効率）

**採用**: アプローチ1（App.tsxで一度取得）
- 理由: パフォーマンスが良い、コードの重複を避ける、キャッシュ効果

## 2. 技術的決定

### 2.1 Headerコンポーネントへのバージョンプロップ追加

**決定事項**:
```typescript
export interface HeaderProps {
  title: string;
  titleColor?: string;
  dividerChar?: string;
  showDivider?: boolean;
  width?: number;
  version?: string | null; // 新規追加
}
```

**レンダリングロジック**:
```tsx
<Text bold color={titleColor}>
  {version ? `${title} v${version}` : title}
</Text>
```

**考慮事項**:
- `version`が`null`の場合はタイトルのみ表示
- `version`が`undefined`の場合もタイトルのみ表示（後方互換性）
- バージョン形式は`"v1.12.3"`のようにプレフィックス付き

### 2.2 CLI引数の検出とバージョン表示関数

**決定事項**:
```typescript
async function showVersion(): Promise<void> {
  const version = await getPackageVersion();
  if (version) {
    console.log(version);
  } else {
    console.error("Error: Unable to retrieve version information");
    process.exit(1);
  }
}
```

**CLI引数パース**:
```typescript
const args = process.argv.slice(2);
const showVersionFlag = args.includes("-v") || args.includes("--version");
const showHelpFlag = args.includes("-h") || args.includes("--help");

if (showVersionFlag) {
  await showVersion();
  return;
}

if (showHelpFlag) {
  showHelp();
  return;
}
```

**考慮事項**:
- `--version`フラグは`--help`よりも優先度が高い（標準的なCLIパターン）
- バージョン表示後は即座に終了（他の処理を実行しない）
- エラー時はexit code 1で終了

### 2.3 エラーハンドリング戦略

**package.json読み取り失敗時の対応**:

1. **CLIフラグの場合**:
   - エラーメッセージを標準エラー出力に表示
   - `process.exit(1)`で異常終了
   - メッセージ例: "Error: Unable to retrieve version information"

2. **UIヘッダーの場合**:
   - バージョンを表示しない（タイトルのみ）
   - ユーザーには通知しない（バージョンは補助情報なので）
   - アプリケーションは正常に動作し続ける

**実装**:
```typescript
// App.tsx
const [version, setVersion] = useState<string | null>(null);

useEffect(() => {
  getPackageVersion().then(setVersion).catch(() => setVersion(null));
}, []);

// Header に渡す
<BranchListScreen version={version} ... />
```

### 2.4 バージョン表示フォーマット

**決定事項**:

1. **CLIフラグ出力**:
   - フォーマット: `"1.12.3"` （バージョン番号のみ）
   - プレフィックスなし（標準的なCLI動作）

2. **UIヘッダー表示**:
   - フォーマット: `"Claude Worktree v1.12.3"` （タイトル + "v" + バージョン）
   - 読みやすさを重視

**実装例**:
```typescript
// CLI
console.log(version); // "1.12.3"

// UI
<Text>{version ? `${title} v${version}` : title}</Text>
// "Claude Worktree v1.12.3"
```

## 3. 制約と依存関係

### 3.1 既存のHeaderコンポーネントAPIとの互換性

**制約**:
- 既存の全7画面でHeaderを使用しているため、後方互換性が必須
- `version`プロップはオプショナルにする必要がある

**解決策**:
- `version?: string | null`をオプショナルプロップとして定義
- `version`が未提供の場合は既存の動作を維持

**検証**:
- 既存の画面コンポーネントでの動作確認
- `React.memo`の動作確認（不要な再レンダリングが発生しないか）

### 3.2 React.memoによる最適化への影響

**懸念事項**:
- `version`プロップの追加が`React.memo`の最適化に影響するか？

**分析**:
- `React.memo`はpropsのshallow comparisonを実行
- `version`が変更された場合のみ再レンダリングが発生
- バージョンは起動時に一度だけ取得されるため、再レンダリングは最小限

**結論**:
- 影響は最小限（パフォーマンス問題なし）
- `React.memo`の比較関数をカスタマイズする必要なし

### 3.3 Ink.jsのレンダリングパフォーマンスへの影響

**懸念事項**:
- バージョン情報の追加がTUIレンダリングに影響するか？

**分析**:
- Headerコンポーネントのレンダリングは軽量（単純なテキスト表示のみ）
- バージョン文字列の追加は最大10文字程度（例: " v1.12.3"）
- Ink.jsのレンダリングエンジンに負荷を与えない

**結論**:
- パフォーマンスへの影響は無視できるレベル
- 追加の最適化は不要

### 3.4 package.jsonの配置場所の仮定

**現在の仮定**:
- package.jsonは`src/`の1つ上のディレクトリに存在
- `getCurrentDirname()`が正しく動作する

**検証**:
- 既存の`getPackageVersion()`関数が正常に動作していることを確認
- ビルド後の実行環境でもパス解決が正しいことを確認

**リスク**:
- ビルド後の`dist/`ディレクトリからの相対パス解決が失敗する可能性
- Bunランタイムでの`import.meta.url`の動作

**緩和策**:
- 既存の実装を信頼（既に本番環境で動作中）
- エラーハンドリングが適切に実装されている

## 4. ベストプラクティスと推奨事項

### 4.1 TypeScript型定義

**推奨**:
- すべての新規関数・インターフェースに適切な型定義を追加
- `string | null`型を使用してエラー状態を明示
- `async/await`を使用して非同期処理を明確に

**実装例**:
```typescript
export interface HeaderProps {
  title: string;
  titleColor?: string;
  dividerChar?: string;
  showDivider?: boolean;
  width?: number;
  version?: string | null;
}

async function showVersion(): Promise<void> {
  const version = await getPackageVersion();
  // ...
}
```

### 4.2 テスト戦略

**推奨テストケース**:

1. **getPackageVersion()のテスト**（既存のテストケースがあれば拡張）:
   - 正常系: package.jsonが存在し、versionフィールドがある
   - 異常系: package.jsonが存在しない
   - 異常系: versionフィールドが存在しない

2. **showVersion()のテスト**:
   - バージョン取得成功時の標準出力
   - バージョン取得失敗時のエラーメッセージ

3. **Headerコンポーネントのテスト**:
   - versionプロップありの場合のレンダリング
   - versionプロップなしの場合のレンダリング
   - versionがnullの場合のレンダリング

### 4.3 ドキュメンテーション

**推奨**:
- `showHelp()`関数のヘルプメッセージに`--version`オプションを追加
- HeaderPropsインターフェースにJSDocコメントを追加
- quickstart.mdにバージョン確認方法を記載

**実装例**:
```typescript
/**
 * Display application version
 * Reads version from package.json and outputs to stdout
 * Exits with code 1 if version cannot be retrieved
 */
async function showVersion(): Promise<void> {
  // ...
}
```

### 4.4 コードスタイルと一貫性

**推奨**:
- 既存のコードスタイルに従う（Chalk、TypeScript、Ink.js）
- `printError()`や`printInfo()`関数を活用（index.tsに既存）
- エラーメッセージは既存のパターンに倣う

**実装例**:
```typescript
function showVersion(): void {
  const version = await getPackageVersion();
  if (version) {
    console.log(version);
  } else {
    printError("Unable to retrieve version information");
    process.exit(1);
  }
}
```

## 5. 実装優先順位

### Phase 1: CLIフラグ実装（P1）

1. `showVersion()`関数の実装（index.ts）
2. CLI引数パース処理の修正（`--version` / `-v`検出）
3. `showHelp()`のヘルプメッセージ更新
4. ユニットテストの実装
5. 統合テストの実装

### Phase 2: UIヘッダー実装（P2）

1. HeaderPropsインターフェースの拡張（Header.tsx）
2. Headerコンポーネントのレンダリングロジック修正
3. App.tsxでのバージョン取得と状態管理
4. 各画面コンポーネントへのversionプロップ追加
5. UIテストの実装

## 6. 技術的リスクと緩和策

### リスク1: package.jsonのパス解決失敗

**リスク**: ビルド後の環境でpackage.jsonの相対パスが異なる可能性

**確率**: 低（既存の`getPackageVersion()`が動作している）

**影響**: バージョン表示が失敗する

**緩和策**:
- 既存の実装を信頼
- エラーハンドリングが適切に実装されている
- テスト環境とビルド環境で動作確認

### リスク2: Headerコンポーネントの再レンダリング

**リスク**: versionプロップの追加が不要な再レンダリングを引き起こす

**確率**: 低（`React.memo`が適切に機能している）

**影響**: パフォーマンス低下

**緩和策**:
- `version`は起動時に一度だけ取得
- `React.memo`の動作を保持
- パフォーマンステストで確認

### リスク3: CLI引数パースの競合

**リスク**: 既存の引数パース処理と`--version`フラグが競合する

**確率**: 極低（シンプルな実装）

**影響**: CLI動作が不安定になる

**緩和策**:
- 既存の`--help`パターンに倣う
- 早期リターンで他の処理をスキップ
- 統合テストで動作確認

## 7. まとめ

### 主要な技術的決定

1. **既存の`getPackageVersion()`関数を再利用**: 新規実装不要、既に動作確認済み
2. **Headerコンポーネントにオプショナルな`version`プロップを追加**: 後方互換性維持
3. **App.tsxレベルでバージョンを一度取得**: パフォーマンスとコードの重複を回避
4. **CLI引数パースは既存のパターンに倣う**: 一貫性とシンプルさ

### 実装の複雑度

**評価**: 低
- 既存の関数を再利用
- 最小限のコード変更
- 後方互換性を維持
- エラーハンドリングが適切

### 次のステップ

- Phase 1（data-model.md、quickstart.md、contracts/）の作成
- `/speckit.tasks`でタスクリストを生成
- `/speckit.implement`で実装を開始

---

**最終更新**: 2025-10-31
**レビュー**: 技術的決定はすべて既存のコードベースパターンに基づいており、実装可能
