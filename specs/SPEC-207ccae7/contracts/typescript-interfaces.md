# TypeScript インターフェース契約

**仕様ID**: `SPEC-207ccae7` | **日付**: 2025-10-31
**目的**: バージョン表示機能のTypeScript型定義を定義する

## 概要

この文書は、バージョン表示機能で使用されるTypeScriptインターフェースと型定義を定義します。

## 1. HeaderProps インターフェース

### 場所

`src/ui/components/parts/Header.tsx`

### 定義

```typescript
export interface HeaderProps {
  /**
   * ヘッダーのタイトル文字列
   * 例: "Claude Worktree - Branch Selection"
   */
  title: string;

  /**
   * タイトルの色（Chalkカラー名）
   * @default 'cyan'
   */
  titleColor?: string;

  /**
   * 区切り線に使用する文字
   * @default '─'
   */
  dividerChar?: string;

  /**
   * 区切り線を表示するか
   * @default true
   */
  showDivider?: boolean;

  /**
   * 区切り線の幅（文字数）
   * @default 80
   */
  width?: number;

  /**
   * アプリケーションのバージョン文字列
   * - string: バージョンが利用可能（例: "1.12.3"）
   * - null: バージョン取得失敗
   * - undefined: バージョン未提供（後方互換性のため）
   * @default undefined
   */
  version?: string | null;
}
```

### 使用例

```typescript
// バージョンあり
<Header
  title="Claude Worktree"
  titleColor="cyan"
  version="1.12.3"
/>
// 表示: "Claude Worktree v1.12.3"

// バージョンなし（後方互換性）
<Header
  title="Claude Worktree"
  titleColor="cyan"
/>
// 表示: "Claude Worktree"

// バージョン取得失敗
<Header
  title="Claude Worktree"
  titleColor="cyan"
  version={null}
/>
// 表示: "Claude Worktree"
```

### バリデーション

- `title`: 必須、空文字列は許可されるが推奨されない
- `titleColor`: オプショナル、Chalkでサポートされる色名
- `dividerChar`: オプショナル、任意の文字列（通常は1文字）
- `showDivider`: オプショナル、boolean値
- `width`: オプショナル、正の整数
- `version`: オプショナル、`string | null` 型

## 2. PackageJson インターフェース

### 場所

`src/utils.ts`

### 定義

```typescript
interface PackageJson {
  /**
   * package.jsonのバージョンフィールド
   * セマンティックバージョニング形式を想定
   * 例: "1.12.3", "2.0.0-beta.1"
   */
  version: string;

  /**
   * package.jsonの名前フィールド（オプショナル）
   * 例: "@akiojin/claude-worktree"
   */
  name?: string;
}
```

### 使用例

```typescript
const packageJson: PackageJson = {
  version: "1.12.3",
  name: "@akiojin/claude-worktree"
};

const packageJsonMinimal: PackageJson = {
  version: "1.12.3"
};
```

### バリデーション

- `version`: 必須、文字列型
- `name`: オプショナル、文字列型

## 3. getPackageVersion() 関数シグネチャ

### 場所

`src/utils.ts`

### 定義

```typescript
/**
 * package.jsonからバージョン情報を取得する
 * @returns バージョン文字列（成功時）、null（失敗時）
 * @throws なし（エラーはnullで表現）
 */
export async function getPackageVersion(): Promise<string | null>;
```

### 戻り値

| 戻り値 | 意味 | 例 |
|--------|------|-----|
| `string` | バージョン取得成功 | `"1.12.3"`, `"2.0.0-beta.1"` |
| `null` | バージョン取得失敗 | package.jsonが存在しない、versionフィールドがない、など |

### 使用例

```typescript
const version = await getPackageVersion();

if (version !== null) {
  console.log(`Version: ${version}`);
} else {
  console.error("Unable to retrieve version");
}
```

## 4. showVersion() 関数シグネチャ

### 場所

`src/index.ts`

### 定義

```typescript
/**
 * バージョン情報を標準出力に表示する
 * バージョン取得失敗時はエラーメッセージを表示して終了
 * @returns Promise<void>
 * @throws なし（エラー時はprocess.exit(1)）
 */
async function showVersion(): Promise<void>;
```

### 動作

1. `getPackageVersion()`を呼び出す
2. 成功時: バージョンを標準出力に出力
3. 失敗時: エラーメッセージを標準エラー出力に出力 + `process.exit(1)`

### 使用例

```typescript
// index.ts main()関数内
const showVersionFlag = args.includes("-v") || args.includes("--version");

if (showVersionFlag) {
  await showVersion();
  return; // ここで早期終了
}
```

## 5. CLI引数型定義

### 定義

```typescript
/**
 * サポートされるCLI引数
 */
type CLIFlag = "--version" | "-v" | "--help" | "-h";

/**
 * CLI引数の配列
 */
type CLIArgs = string[];
```

### 使用例

```typescript
const args: CLIArgs = process.argv.slice(2);
const showVersionFlag: boolean = args.includes("-v") || args.includes("--version");
```

## 6. Reactコンポーネント型定義

### Header コンポーネント

```typescript
/**
 * Headerコンポーネント
 * React.memoで最適化されている
 */
export const Header: React.NamedExoticComponent<HeaderProps>;
```

### 使用例

```typescript
import { Header } from './parts/Header';

function MyScreen() {
  const version = "1.12.3";

  return (
    <Box>
      <Header title="My Screen" version={version} />
    </Box>
  );
}
```

## 7. エラー型定義

### エラーハンドリング

```typescript
/**
 * package.json読み取りエラー
 * getPackageVersion()内で処理され、nullを返す
 */
type PackageJsonError =
  | "FileNotFound"        // package.jsonが存在しない
  | "InvalidJSON"         // JSONパースエラー
  | "MissingVersion"      // versionフィールドがない
  | "PathResolutionError" // パス解決エラー
  | "ReadError";          // ファイル読み取りエラー
```

### 注意

- 実際のコードでは、すべてのエラーを`catch`ブロックでキャッチし、`null`を返します
- `PackageJsonError`型は説明目的のみで、実装には使用しません

## 8. 型ガード

### versionの型ガード

```typescript
/**
 * versionがnullまたはundefinedでないことを確認
 * @param version - チェックするバージョン
 * @returns versionが有効な文字列の場合true
 */
function isValidVersion(version: string | null | undefined): version is string {
  return version !== null && version !== undefined;
}

// 使用例
if (isValidVersion(version)) {
  // この中では version は string 型
  console.log(`Version: ${version}`);
}
```

## 9. 型の互換性

### 後方互換性

**重要**: `version`プロップはオプショナルであり、既存のコードとの互換性を保証します。

```typescript
// 既存のコード（修正不要）
<Header title="Claude Worktree" />

// 新しいコード
<Header title="Claude Worktree" version="1.12.3" />
```

### 型チェック

TypeScriptコンパイラは以下をチェックします：

- ✅ `title`が提供されている
- ✅ `titleColor`が文字列型（提供された場合）
- ✅ `version`が`string | null | undefined`型（提供された場合）
- ❌ `version`に数値を渡す（コンパイルエラー）

## 10. まとめ

### 主要な型定義

| 型/インターフェース | 場所 | 説明 |
|------------------|------|------|
| `HeaderProps` | `Header.tsx` | Headerコンポーネントのプロップス |
| `PackageJson` | `utils.ts` | package.jsonの型定義 |
| `getPackageVersion()` | `utils.ts` | バージョン取得関数 |
| `showVersion()` | `index.ts` | バージョン表示関数 |

### 型安全性の保証

- すべての関数とインターフェースに型定義が付与されている
- 非同期処理は`Promise`型で明示されている
- エラー処理は`null`型で表現されている
- オプショナルプロップは`?`で明示されている

### 次のステップ

- 実装時にこれらの型定義に従う
- TypeScriptコンパイラの警告に注意
- `strict`モードでコンパイルしてエラーがないことを確認

---

**最終更新**: 2025-10-31
**ステータス**: 契約定義完了
