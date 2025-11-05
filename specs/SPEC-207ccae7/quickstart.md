# クイックスタートガイド: アプリケーションバージョン表示機能

**仕様ID**: `SPEC-207ccae7` | **日付**: 2025-10-31
**対象読者**: 開発者、テスター
**目的**: バージョン表示機能の開発とテストを迅速に開始するためのガイド

## 📋 前提条件

以下がインストールされていることを確認してください：

- **Bun**: v1.0以降（推奨ランタイム）
- **Node.js**: v18以降（Bunの代替として使用可能）
- **Git**: バージョン管理用

## 🚀 セットアップ手順

### 1. リポジトリのクローンとブランチ切り替え

```bash
# リポジトリをクローン（まだの場合）
git clone https://github.com/akiojin/claude-worktree.git
cd claude-worktree

# feature/show-versionブランチに切り替え
git checkout feature/show-version

# 最新の変更を取得
git pull origin feature/show-version
```

### 2. 依存関係のインストール

```bash
# Bunを使用（推奨）
bun install

# または、npmを使用
# npm install
```

### 3. ビルド

```bash
# TypeScriptをビルド
bun run build
```

**ビルド成果物**: `dist/` ディレクトリに生成されます

### 4. 実行

#### 方法1: ローカル実行

```bash
# バージョン表示
bunx . --version
bunx . -v

# ヘルプ表示
bunx . --help

# メインUI起動
bunx .
```

#### 方法2: グローバルインストール

```bash
# グローバルにインストール
bun add -g @akiojin/claude-worktree

# 実行
claude-worktree --version
claude-worktree
```

## 🔧 開発ワークフロー

### 開発サイクル

```bash
# 1. コードを編集
vim src/index.ts

# 2. ビルド
bun run build

# 3. テスト実行（実装後）
bun test

# 4. 動作確認
bunx . --version
```

### ホットリロード開発

```bash
# 開発モード（ファイル監視）
bun run dev

# 別ターミナルで実行
bunx . --version
```

## 📝 よくある操作

### バージョン表示機能のテスト

#### CLI フラグでのバージョン表示

```bash
# --versionフラグ
bunx . --version
# 出力例: 1.12.3

# -vフラグ（ショート形式）
bunx . -v
# 出力例: 1.12.3
```

**期待される動作**:
- バージョン番号のみを標準出力に出力
- 即座に終了（他の処理を実行しない）
- 終了コード: 0（成功）

#### UIヘッダーでのバージョン表示

```bash
# メインUIを起動
bunx .
```

**期待される動作**:
- ブランチ一覧画面のヘッダーに「Claude Worktree v1.12.3」と表示される
- すべての画面でヘッダーにバージョンが表示される

### エラーハンドリングのテスト

#### package.jsonが存在しない場合

```bash
# package.jsonを一時的にリネーム
mv package.json package.json.bak

# バージョン表示を実行
bunx . --version
# 出力例: Error: Unable to retrieve version information
# 終了コード: 1

# package.jsonを復元
mv package.json.bak package.json
```

#### versionフィールドが存在しない場合

```bash
# package.jsonを編集してversionフィールドを削除
vim package.json

# バージョン表示を実行
bunx . --version
# 出力例: Error: Unable to retrieve version information

# package.jsonを元に戻す
git checkout package.json
```

## 🧪 テスト実行

### ユニットテスト

```bash
# すべてのテストを実行
bun test

# 特定のテストファイルを実行
bun test src/utils.test.ts

# カバレッジ付きで実行
bun test --coverage
```

### 統合テスト

```bash
# CLIフラグのテスト
bun test src/index.test.ts

# Headerコンポーネントのテスト
bun test src/ui/components/parts/Header.test.tsx
```

### エンドツーエンドテスト

```bash
# 実際のCLI実行をテスト
bun run build
bunx . --version
```

## 🐛 トラブルシューティング

### 問題1: `bunx . --version` が動作しない

**症状**: コマンドを実行してもバージョンが表示されない

**原因**: ビルドされていない

**解決策**:
```bash
bun run build
bunx . --version
```

### 問題2: `Error: Unable to retrieve version information`

**症状**: バージョン取得エラーが表示される

**原因**: package.jsonが見つからない、またはversionフィールドがない

**解決策**:
```bash
# package.jsonの存在確認
ls -la package.json

# package.jsonの内容確認
cat package.json | grep version

# リポジトリルートにいることを確認
pwd
```

### 問題3: UIでバージョンが表示されない

**症状**: メインUIのヘッダーにバージョンが表示されない

**原因**: versionプロップが渡されていない、またはgetPackageVersion()がnullを返している

**デバッグ方法**:
```bash
# デバッグモードで実行
DEBUG=1 bunx .

# コンソールでversionの値を確認
# App.tsx内でconsole.log(version)を追加
```

### 問題4: Bun実行時にエラーが発生する

**症状**: `bun: command not found`

**原因**: Bunがインストールされていない

**解決策**:
```bash
# Bunをインストール
curl -fsSL https://bun.sh/install | bash

# パスを確認
source ~/.bashrc  # または ~/.zshrc

# バージョン確認
bun --version
```

### 問題5: Node.jsで実行したい

**症状**: Bunではなく、Node.jsで実行したい

**解決策**:
```bash
# ビルド
npm run build

# 実行
node dist/index.js --version

# または、npxを使用
npx . --version
```

## 📁 ファイル構造

### 修正が必要なファイル

```
src/
├── index.ts                           # CLIエントリーポイント（修正対象）
│   ├── showVersion()関数を追加
│   └── CLI引数パースを修正
│
├── utils.ts                           # ユーティリティ関数（既存）
│   └── getPackageVersion()関数（再利用）
│
└── ui/
    └── components/
        ├── App.tsx                    # メインアプリケーション（修正対象）
        │   └── バージョン取得とstate管理を追加
        │
        ├── parts/
        │   └── Header.tsx             # ヘッダーコンポーネント（修正対象）
        │       └── versionプロップを追加
        │
        └── screens/
            ├── BranchListScreen.tsx   # 各画面（修正対象）
            ├── BranchCreatorScreen.tsx
            ├── WorktreeManagerScreen.tsx
            └── （その他5画面）
                └── versionプロップを渡す
```

### テストファイル

```
src/
├── utils.test.ts                      # getPackageVersion()のテスト
├── index.test.ts                      # showVersion()のテスト
└── ui/
    └── components/
        └── parts/
            └── Header.test.tsx        # Headerコンポーネントのテスト
```

## 🔍 コードレビューチェックリスト

実装完了後、以下を確認してください：

- [ ] `showVersion()`関数が実装されている
- [ ] CLI引数パースが`--version`と`-v`をサポート
- [ ] `showHelp()`のヘルプメッセージが更新されている
- [ ] HeaderPropsに`version`プロップが追加されている
- [ ] Headerコンポーネントのレンダリングロジックが更新されている
- [ ] App.tsxでバージョン取得と状態管理が実装されている
- [ ] すべての画面コンポーネントが`version`プロップを受け取る
- [ ] ユニットテストがパスする
- [ ] 統合テストがパスする
- [ ] エラーハンドリングが適切に実装されている
- [ ] TypeScript型定義が正しい
- [ ] ドキュメントが更新されている

## 📚 追加リソース

### 関連ドキュメント

- [機能仕様書](./spec.md) - 機能の詳細仕様
- [実装計画](./plan.md) - 実装戦略とアーキテクチャ
- [調査レポート](./research.md) - 既存コードベースの分析
- [データモデル](./data-model.md) - データ構造と型定義

### 外部リソース

- [Bun公式ドキュメント](https://bun.sh/docs)
- [Ink.js公式ドキュメント](https://github.com/vadimdemedes/ink)
- [セマンティックバージョニング](https://semver.org/)
- [TypeScript公式ドキュメント](https://www.typescriptlang.org/)

## 💡 開発のヒント

### ヒント1: ローカル開発での反復

```bash
# 1回のコマンドでビルドと実行
bun run build && bunx . --version
```

### ヒント2: デバッグログの追加

```typescript
// utils.ts
export async function getPackageVersion(): Promise<string | null> {
  try {
    const currentDir = getCurrentDirname();
    const packageJsonPath = path.resolve(currentDir, "..", "package.json");

    console.log(`[DEBUG] Reading package.json from: ${packageJsonPath}`);

    const packageJsonContent = await readFile(packageJsonPath, "utf-8");
    const packageJson: PackageJson = JSON.parse(packageJsonContent);

    console.log(`[DEBUG] Version: ${packageJson.version}`);

    return packageJson.version || null;
  } catch (error) {
    console.error(`[DEBUG] Error: ${error}`);
    return null;
  }
}
```

### ヒント3: 環境変数でデバッグモード

```bash
# デバッグモードで実行
DEBUG=1 bunx . --version

# 実装例（index.ts）
if (process.env.DEBUG) {
  console.log('[DEBUG] CLI args:', args);
}
```

### ヒント4: パフォーマンス測定

```typescript
const start = performance.now();
const version = await getPackageVersion();
const end = performance.now();
console.log(`Version fetch took ${end - start}ms`);
```

## 🎯 次のステップ

1. ✅ このガイドを読み終えた
2. ⏭️ [タスクリスト](./tasks.md)を確認（`/speckit.tasks`実行後）
3. ⏭️ 実装を開始（`/speckit.implement`実行）
4. ⏭️ テストを実行してすべてパスすることを確認
5. ⏭️ コードレビューチェックリストを確認
6. ⏭️ プルリクエストを作成

## 📞 サポート

質問や問題がある場合は、以下を確認してください：

- [GitHubイシュー](https://github.com/akiojin/claude-worktree/issues)
- [CLAUDE.md](../../CLAUDE.md) - プロジェクト全体の開発指針
- [README.md](../../README.md) - プロジェクト概要

---

**最終更新**: 2025-10-31
**メンテナー**: Claude Code
**ステータス**: 準備完了
