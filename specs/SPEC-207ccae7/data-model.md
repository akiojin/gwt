# データモデル設計: アプリケーションバージョン表示機能

**仕様ID**: `SPEC-207ccae7` | **日付**: 2025-10-31
**目的**: バージョン表示機能の主要エンティティとデータ構造を定義する

## 概要

この機能は主に以下のデータエンティティを扱います：

1. **バージョン情報**: package.jsonから取得されるバージョン文字列
2. **HeaderPropsインターフェース**: UIヘッダーコンポーネントのプロップス
3. **CLI引数**: コマンドライン引数の構造

## 1. バージョン情報エンティティ

### 1.1 データソース

**ソースファイル**: `package.json`

**フィールド**: `version`

**フォーマット**: セマンティックバージョニング（MAJOR.MINOR.PATCH）

**例**:
```json
{
  "name": "@akiojin/claude-worktree",
  "version": "1.12.3"
}
```

### 1.2 TypeScript型定義

**既存の型定義**（`src/utils.ts`）:
```typescript
interface PackageJson {
  version: string;
  name?: string;
}
```

**取得関数の戻り値型**:
```typescript
async function getPackageVersion(): Promise<string | null>
```

**型の意味**:
- `string`: バージョン取得成功（例: `"1.12.3"`）
- `null`: バージョン取得失敗（package.jsonが存在しない、versionフィールドがない、など）

### 1.3 バージョン文字列の形式

**フォーマット**:
```
MAJOR.MINOR.PATCH
```

**例**:
- `"1.12.3"`
- `"2.0.0"`
- `"0.1.0-beta"`

**検証ルール**:
- セマンティックバージョニング仕様に準拠
- 最小形式: `X.Y.Z` （X, Y, Z は非負整数）
- オプショナル: プレリリース識別子（例: `-beta`, `-alpha.1`）
- オプショナル: ビルドメタデータ（例: `+20130313144700`）

**注**: この機能ではバージョン文字列の検証は行わず、package.jsonの値をそのまま使用します。

### 1.4 ライフサイクル

**取得タイミング**:
1. **CLIフラグの場合**: `--version`/`-v`フラグ検出時に取得
2. **UIヘッダーの場合**: App.tsxのuseEffect内で起動時に一度取得

**キャッシング**:
- **CLIフラグ**: キャッシュなし（即座に取得して表示）
- **UIヘッダー**: App.tsxのstateに保存（再取得なし）

**更新**:
- アプリケーションのライフサイクル中はバージョンは不変
- package.jsonが更新された場合、アプリケーション再起動が必要

## 2. HeaderPropsインターフェース

### 2.1 現在の定義

**場所**: `src/ui/components/parts/Header.tsx`

**現在の構造**:
```typescript
export interface HeaderProps {
  title: string;
  titleColor?: string;
  dividerChar?: string;
  showDivider?: boolean;
  width?: number;
}
```

### 2.2 拡張後の定義

**新しい構造**:
```typescript
export interface HeaderProps {
  title: string;
  titleColor?: string;
  dividerChar?: string;
  showDivider?: boolean;
  width?: number;
  version?: string | null;  // 新規追加
}
```

### 2.3 フィールド仕様

#### 既存フィールド

| フィールド | 型 | 必須 | デフォルト | 説明 |
|----------|------|------|-----------|------|
| `title` | `string` | ✅ | - | ヘッダーのタイトル文字列 |
| `titleColor` | `string` | ❌ | `'cyan'` | タイトルの色（Chalkカラー名） |
| `dividerChar` | `string` | ❌ | `'─'` | 区切り線に使用する文字 |
| `showDivider` | `boolean` | ❌ | `true` | 区切り線を表示するか |
| `width` | `number` | ❌ | `80` | 区切り線の幅（文字数） |

#### 新規フィールド

| フィールド | 型 | 必須 | デフォルト | 説明 |
|----------|------|------|-----------|------|
| `version` | `string \| null` | ❌ | `undefined` | アプリケーションのバージョン文字列 |

**`version`フィールドの動作**:

| 値 | 表示結果 | 例 |
|----|---------|-----|
| `"1.12.3"` | `"Claude Worktree v1.12.3"` | タイトル + " v" + バージョン |
| `null` | `"Claude Worktree"` | タイトルのみ（バージョン取得失敗） |
| `undefined` | `"Claude Worktree"` | タイトルのみ（後方互換性） |

### 2.4 バリデーション

**実行時検証**:
- なし（TypeScriptの型チェックのみ）

**型ガード**:
```typescript
if (version !== null && version !== undefined) {
  // バージョンを表示
}
```

### 2.5 後方互換性

**保証事項**:
- `version`プロップが未提供の場合、既存の動作を維持
- 既存の全7画面コンポーネントで修正なしに動作

## 3. CLI引数構造

### 3.1 引数の形式

**フォーマット**:
```bash
claude-worktree [options]
```

**サポートされる引数**:

| 引数 | ショート形式 | 説明 | 優先度 |
|------|------------|------|--------|
| `--version` | `-v` | バージョンを表示して終了 | 1（最優先） |
| `--help` | `-h` | ヘルプメッセージを表示して終了 | 2 |

### 3.2 引数の優先順位

**処理順序**:
```
1. --version / -v  → showVersion() → process.exit(0)
2. --help / -h     → showHelp()    → return
3. その他          → メインUI起動
```

**理由**: 標準的なCLI動作に準拠（`--version`は最優先）

### 3.3 引数パース実装

**場所**: `src/index.ts:main()`

**現在の実装**:
```typescript
const args = process.argv.slice(2);
const showHelpFlag = args.includes("-h") || args.includes("--help");

if (showHelpFlag) {
  showHelp();
  return;
}
```

**拡張後の実装**:
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

### 3.4 引数の組み合わせ

**複数引数の同時指定**:
```bash
claude-worktree --version --help
```

**動作**: `--version`のみ処理（優先度順）

**理由**: 早期リターンにより、後続の引数は処理されない

## 4. データフロー

### 4.1 CLIフラグでのデータフロー

```
1. ユーザーが`claude-worktree --version`を実行
   ↓
2. main()関数が`--version`フラグを検出
   ↓
3. showVersion()関数を呼び出し
   ↓
4. getPackageVersion()を呼び出し
   ↓
5. package.jsonを読み取り、versionフィールドを取得
   ↓
6. バージョン文字列をstdoutに出力
   ↓
7. process.exit(0)で終了
```

**エラーフロー**:
```
4. getPackageVersion()がnullを返す
   ↓
5. エラーメッセージをstderrに出力
   ↓
6. process.exit(1)で異常終了
```

### 4.2 UIヘッダーでのデータフロー

```
1. アプリケーション起動（App.tsx）
   ↓
2. useEffect()内でgetPackageVersion()を呼び出し
   ↓
3. バージョン取得成功 → state更新（version: "1.12.3"）
   バージョン取得失敗 → state更新（version: null）
   ↓
4. 各画面コンポーネントにversionをpropsとして渡す
   ↓
5. Headerコンポーネントがversionを受け取る
   ↓
6. version !== null ? `${title} v${version}` : title
   ↓
7. 画面にレンダリング
```

## 5. エラー処理

### 5.1 エラーシナリオ

| シナリオ | 原因 | 検出方法 | 処理 |
|---------|------|---------|------|
| package.json不在 | ファイルが存在しない | `readFile()`の例外 | `null`を返す |
| versionフィールド不在 | JSONにversionがない | `packageJson.version`が`undefined` | `null`を返す |
| JSON解析エラー | 不正なJSON | `JSON.parse()`の例外 | `null`を返す |
| パス解決エラー | 相対パスが不正 | `path.resolve()`の例外 | `null`を返す |

### 5.2 エラーハンドリング戦略

**getPackageVersion()関数**:
```typescript
export async function getPackageVersion(): Promise<string | null> {
  try {
    // バージョン取得処理
    return packageJson.version || null;
  } catch {
    // すべてのエラーをキャッチしてnullを返す
    return null;
  }
}
```

**showVersion()関数**:
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

**Headerコンポーネント**:
```typescript
<Text bold color={titleColor}>
  {version ? `${title} v${version}` : title}
</Text>
```

### 5.3 ロギング

**推奨**: エラー情報を詳細にログ出力しない

**理由**:
- セキュリティ（ファイルパスの漏洩を防ぐ）
- ユーザーフレンドリー（技術的詳細を隠す）
- シンプルさ（複雑なロギング機構不要）

**例外**: デバッグモードでは詳細ログを出力可能

## 6. パフォーマンス考慮事項

### 6.1 ファイルI/O

**package.json読み取り**:
- **頻度**: アプリケーション起動時に1回のみ
- **サイズ**: 通常1KB未満
- **影響**: 無視できるレベル

### 6.2 メモリ使用量

**バージョン文字列**:
- **サイズ**: 約10バイト（`"1.12.3"`）
- **保存場所**: App.tsxのstate
- **影響**: 無視できるレベル

### 6.3 レンダリング

**Headerコンポーネント**:
- **追加文字数**: 最大15文字（`" v1.12.3"`）
- **再レンダリング**: 起動時のみ（`React.memo`により最適化）
- **影響**: 無視できるレベル

## 7. テストデータ

### 7.1 正常系テストデータ

**package.json（正常）**:
```json
{
  "name": "@akiojin/claude-worktree",
  "version": "1.12.3"
}
```

**期待される結果**:
- CLI: `"1.12.3"`
- UI: `"Claude Worktree v1.12.3"`

### 7.2 異常系テストデータ

**package.json（versionなし）**:
```json
{
  "name": "@akiojin/claude-worktree"
}
```

**期待される結果**:
- CLI: エラーメッセージ + exit 1
- UI: `"Claude Worktree"`（バージョンなし）

**package.json（不正なJSON）**:
```json
{
  "name": "@akiojin/claude-worktree",
  "version": "1.12.3",
}
```
（末尾のカンマが不正）

**期待される結果**:
- CLI: エラーメッセージ + exit 1
- UI: `"Claude Worktree"`（バージョンなし）

### 7.3 エッジケースデータ

**package.json（プレリリースバージョン）**:
```json
{
  "name": "@akiojin/claude-worktree",
  "version": "2.0.0-beta.1"
}
```

**期待される結果**:
- CLI: `"2.0.0-beta.1"`
- UI: `"Claude Worktree v2.0.0-beta.1"`

## 8. セキュリティ考慮事項

### 8.1 ファイル読み取り

**脅威**: パストラバーサル攻撃

**緩和策**:
- 相対パスを使用（`path.resolve(currentDir, "..", "package.json")`）
- ユーザー入力を受け付けない
- 読み取り専用操作のみ

### 8.2 エラーメッセージ

**脅威**: ファイルパスの漏洩

**緩和策**:
- エラーメッセージに具体的なパスを含めない
- 一般的なエラーメッセージを使用

## 9. まとめ

### 主要なデータエンティティ

1. **バージョン情報**: `string | null` 型、package.jsonから取得
2. **HeaderProps**: `version?: string | null` プロップを追加
3. **CLI引数**: `--version` / `-v` フラグをサポート

### データフローの特徴

- **シンプル**: 複雑な状態管理不要
- **効率的**: 起動時に一度だけ取得
- **安全**: 適切なエラーハンドリング

### 次のステップ

- quickstart.mdの作成
- contracts/の作成（TypeScript型定義を含む）
- `/speckit.tasks`でタスクリストを生成

---

**最終更新**: 2025-10-31
**ステータス**: 設計完了、実装準備完了
