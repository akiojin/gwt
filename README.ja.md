# @akiojin/claude-worktree

[English](README.md)

Claude Code用の対話型Gitワークツリーマネージャー（グラフィカルなブランチ選択と高度なワークフロー管理機能付き）

## 概要

`@akiojin/claude-worktree`は、直感的なインターフェースを通じてGitワークツリー管理を革新する強力なCLIツールです。Claude Code開発ワークフローとシームレスに統合し、インテリジェントなブランチ選択、自動ワークツリー作成、包括的なプロジェクト管理機能を提供します。

## ✨ 主要機能

- 🎯 **対話型ブランチ選択**: エレガントなテーブルベースインターフェースでローカル・リモートブランチをナビゲート
- 🌟 **スマートブランチ作成**: ガイド付きプロンプトと自動ベースブランチ選択でfeature、hotfix、releaseブランチを作成
- 🔄 **高度なワークツリー管理**: 作成、クリーンアップ、パス最適化を含む完全なライフサイクル管理
- 🚀 **Claude Code統合**: 権限設定と開発後の変更処理を含むシームレスな起動
- 📊 **GitHub PR統合**: マージされたプルリクエストのブランチとワークツリーの自動クリーンアップ
- 🛠️ **変更管理**: 開発セッション後のコミット、stash、破棄の内蔵サポート
- 📦 **ユニバーサルパッケージ**: 一度インストールすれば全プロジェクトで一貫した動作
- 🔍 **リポジトリ統計**: プロジェクト概要の向上のためのブランチとワークツリー数のリアルタイム表示

## インストール

npmでグローバルインストール:

```bash
npm install -g @akiojin/claude-worktree
```

## クイックスタート

任意のGitリポジトリで実行:

```bash
claude-worktree
```

ツールは以下のオプションを持つ対話型インターフェースを提供します:

1. **既存ブランチを選択**: ワークツリー自動作成機能付きでローカル・リモートブランチから選択
2. **新規ブランチ作成**: タイプ選択（feature/hotfix/release）によるガイド付きブランチ作成
3. **ワークツリー管理**: 既存ワークツリーの表示、オープン、削除
4. **マージ済みPRクリーンアップ**: マージされたGitHubプルリクエストのブランチとワークツリーを自動削除

## 高度なワークフロー

### ブランチ作成ワークフロー

1. メインメニューから「新規ブランチ作成」を選択
2. ブランチタイプ（feature、hotfix、release）を選択
3. 自動プレフィックス適用でブランチ名を入力
4. 利用可能なオプションからベースブランチを選択
5. ワークツリー作成パスを確認
6. 自動ワークツリーセットアップとClaude Code起動

### ワークツリー管理

- **既存を開く**: 既存ワークツリーでClaude Codeを起動
- **ワークツリー削除**: オプションのブランチ削除付きクリーン削除
- **バッチ操作**: 複数ワークツリーの効率的な処理

### GitHub統合

- **マージ済みPRクリーンアップ**: マージされたプルリクエストブランチの自動検出と削除
- **認証確認**: 操作前にGitHub CLIセットアップを検証
- **リモート同期**: クリーンアップ操作前に最新変更を取得

## システム要件

- **Node.js**: >= 18.0.0
- **Git**: ワークツリーサポート付き最新版
- **Claude Code**: 最適な開発体験のため
- **GitHub CLI**: PR クリーンアップ機能に必要（オプション）

## プロジェクト構造

```
@akiojin/claude-worktree/
├── src/
│   ├── index.ts          # メインアプリケーションエントリーポイント
│   ├── git.ts           # Git操作とブランチ管理
│   ├── worktree.ts      # ワークツリー作成と管理
│   ├── claude.ts        # Claude Code統合
│   ├── github.ts        # GitHub CLI統合
│   ├── utils.ts         # ユーティリティ関数とエラーハンドリング
│   └── ui/              # ユーザーインターフェースコンポーネント
│       ├── display.ts   # コンソール出力フォーマット
│       ├── prompts.ts   # 対話型プロンプト
│       ├── table.ts     # ブランチテーブル生成
│       └── types.ts     # TypeScript型定義
├── bin/
│   └── claude-worktree.js # 実行可能ラッパー
└── dist/                # コンパイル済みJavaScript出力
```

## 開発

### セットアップ

```bash
# リポジトリをクローン
git clone https://github.com/akiojin/claude-worktree.git
cd claude-worktree

# 依存関係をインストール
npm install

# プロジェクトをビルド
npm run build
```

### 利用可能なスクリプト

```bash
# 自動リビルド付き開発モード
npm run dev

# プロダクションビルド
npm run build

# 型チェック
npm run type-check

# コードリンティング
npm run lint

# ビルド成果物をクリーン
npm run clean

# CLIをローカルテスト
npm run start
```

### 開発ワークフロー

1. **フォークとクローン**: リポジトリをフォークし、あなたのフォークをクローン
2. **ブランチ作成**: ツール自体を使用してfeatureブランチを作成
3. **開発**: TypeScriptサポート付きで変更を実施
4. **テスト**: `npm run start`でCLI機能をテスト
5. **品質チェック**: `npm run type-check`と`npm run lint`を実行
6. **プルリクエスト**: 明確な説明付きでPRを提出

### コード構造

- **エントリーポイント**: `src/index.ts` - メインアプリケーションロジック
- **コアモジュール**: Git操作、ワークツリー管理、Claude統合
- **UIコンポーネント**: `src/ui/`のモジュラーインターフェースコンポーネント
- **型安全性**: 包括的なTypeScript定義
- **エラーハンドリング**: 全モジュールにわたる堅牢なエラー管理

## 統合例

### CI/CD統合

```yaml
# GitHub Actions例
- name: Setup Worktree
  run: |
    npm install -g @akiojin/claude-worktree
    claude-worktree --help
```

### カスタムスクリプト

```bash
# Package.jsonスクリプト例
{
  "scripts": {
    "worktree": "claude-worktree"
  }
}
```

## トラブルシューティング

### よくある問題

**権限エラー**: Claude Codeが適切なディレクトリ権限を持っていることを確認  
**Git ワークツリー競合**: クリーンアップ機能を使用して古いワークツリーを削除  
**GitHub認証**: PRクリーンアップ機能使用前に`gh auth login`を実行  
**Nodeバージョン**: `node --version`でNode.js >= 18.0.0を確認

### デバッグモード

詳細出力には環境変数を設定:

```bash
DEBUG=claude-worktree claude-worktree
```

## ライセンス

MIT - 詳細はLICENSEファイルを参照

## 貢献

貢献を歓迎します！以下の貢献ガイドラインをお読みください:

1. **Issues**: GitHub IssuesでバグレポートやFeatureリクエストを報告
2. **プルリクエスト**: 上記の開発ワークフローに従う
3. **コードスタイル**: TypeScriptベストプラクティスと既存パターンを維持
4. **ドキュメント**: 重要な変更についてはREADMEとコードコメントを更新

### 貢献者

- AI Novel Project Team
- コミュニティ貢献者歓迎

## サポート

- **ドキュメント**: このREADMEとインラインコードドキュメント
- **Issues**: バグレポートとFeatureリクエスト用のGitHub Issues
- **ディスカッション**: 質問とコミュニティサポート用のGitHub Discussions