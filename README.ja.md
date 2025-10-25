# @akiojin/claude-worktree

[English](README.md)

Claude Code / Codex CLI 対応の対話型Gitワークツリーマネージャー（グラフィカルなブランチ選択と高度なワークフロー管理機能付き）

## 概要

`@akiojin/claude-worktree`は、直感的なインターフェースを通じてGitワークツリー管理を革新する強力なCLIツールです。Claude Code / Codex CLI の開発ワークフローとシームレスに統合し、インテリジェントなブランチ選択、自動ワークツリー作成、包括的なプロジェクト管理機能を提供します。

## ✨ 主要機能

- 🎯 **対話型ブランチ選択**: ブランチ種別・ワークツリー・変更状態アイコンに加え配置インジケータ枠（左=L, 右=R, リモートのみ=☁）で所在を示し、リモート名の `origin/` を省いたシンプルなリストで判別しやすく、現在の選択は `>` プレフィックスで強調されるため誤操作を防止
- 🌟 **スマートブランチ作成**: ガイド付きプロンプトと自動ベースブランチ選択でfeature、hotfix、releaseブランチを作成
- 🔄 **高度なワークツリー管理**: 作成、クリーンアップ、パス最適化を含む完全なライフサイクル管理
- 🤖 **AIツール選択**: 起動時に Claude Code / Codex CLI を選択、または `--tool` で直接指定（`--` 以降は各ツールへ引数パススルー）
- 🚀 **AIツール統合**: 選択したツールをワークツリーで起動（Claude Codeは権限設定・変更処理の統合あり）
- 📊 **GitHub PR統合**: マージされたプルリクエストのブランチとワークツリーの自動クリーンアップ
- 🛠️ **変更管理**: 開発セッション後のコミット、stash、破棄の内蔵サポート
- 📦 **ユニバーサルパッケージ**: 一度インストールすれば全プロジェクトで一貫した動作
- 🔍 **リポジトリ統計**: プロジェクト概要の向上のためのブランチとワークツリー数のリアルタイム表示

## インストール

### グローバルインストール（推奨）

永続的なアクセスのためにグローバルインストール:

#### bun（グローバルインストール）

```bash
bun add -g @akiojin/claude-worktree
```

### 一回限りの使用

インストールせずに実行:

#### bunx (bun)

```bash
bunx @akiojin/claude-worktree
```

## クイックスタート

任意のGitリポジトリで実行:

```bash
# グローバルインストール済みの場合
claude-worktree

# または一回限りの実行に bunx を使用
bunx @akiojin/claude-worktree
```

### AIツール選択と直接指定

```bash
# 対話的にツールを選ぶ（Claude / Codex）
claude-worktree

# 直接指定
claude-worktree --tool claude
claude-worktree --tool codex

# ツール固有オプションを渡す（"--" 以降はツールへパススルー）
claude-worktree --tool claude -- -r      # Claude Code を resume
claude-worktree --tool codex -- resume --last  # Codex CLI の直近セッションを再開
claude-worktree --tool codex -- resume <id>  # Codex CLI の特定セッションを再開
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
6. 自動ワークツリーセットアップと選択ツールの起動

### ワークツリー管理

- **既存を開く**: 既存ワークツリーで選択ツールを起動
- **ワークツリー削除**: オプションのブランチ削除付きクリーン削除
- **バッチ操作**: 複数ワークツリーの効率的な処理

### GitHub統合

- **マージ済みPRクリーンアップ**: マージされたプルリクエストブランチの自動検出と削除
- **認証確認**: 操作前にGitHub CLIセットアップを検証
- **リモート同期**: クリーンアップ操作前に最新変更を取得

### PR自動マージ

リポジトリには、開発プロセスを効率化するPR自動マージワークフローが含まれています：

- **自動マージ**: すべてのCIチェック（Test、Lint）が成功し、競合がない場合、PRを自動的にマージ
- **マージ方法**: マージコミットを使用して完全なコミット履歴を保持
- **スマートスキップロジック**: ドラフトPR、競合のあるPR、CI失敗時は自動的にスキップ
- **対象ブランチ**: `main`および`develop`ブランチへのPRで有効
- **安全第一**: ブランチ保護ルールを尊重し、CI成功を必須条件とする

**動作の仕組み:**

1. `main`または`develop`を対象とするPRを作成
2. CIワークフロー（Test、Lint）が自動実行
3. すべてのCIチェックが成功し、競合がない場合、PRが自動的にマージされる
4. 手動介入は不要 - PRを作成してCIに任せるだけ

**自動マージの無効化:**

- ドラフトPRとして作成すると自動マージを防げます: `gh pr create --draft`
- 自動マージワークフローはこの設定を尊重し、ドラフトPRはスキップします

技術的な詳細については、[specs/SPEC-cff08403/](specs/SPEC-cff08403/)を参照してください。

## システム要件

- **Bun**: >= 1.0.0
- **Node.js**（任意）: Nodeベースの開発ツール利用時は >= 18.0.0 を推奨
- **Git**: ワークツリーサポート付き最新版
- **AIツール**: 少なくともいずれかが必要（Claude Code もしくは Codex CLI）
- **GitHub CLI**: PR クリーンアップ機能に必要（オプション）
- **Python**: >= 3.11（Spec Kit CLIに必要）
- **uv**: Pythonパッケージマネージャー（Spec Kit CLIに必要）

## Spec Kit による仕様駆動開発

このプロジェクトは、仕様駆動開発ワークフローのために **@akiojin/spec-kit**（GitHub Spec Kit の日本語対応版）を使用しています。

### Spec Kit CLI のインストール

```bash
# uvでグローバルインストール
uv tool install specify-cli --from git+https://github.com/akiojin/spec-kit.git

# インストール確認
specify --help
```

### 利用可能な Spec Kit コマンド

Claude Code で以下のコマンドを実行して、仕様駆動開発を活用できます：

- `/speckit.constitution` - プロジェクト原則とガイドラインを定義
- `/speckit.specify` - 機能仕様書を作成
- `/speckit.plan` - 技術実装計画を作成
- `/speckit.tasks` - 実行可能なタスクリストを生成
- `/speckit.implement` - 実装を実行

### 品質保証用オプションコマンド

- `/speckit.clarify` - 計画前に曖昧な領域を解消
- `/speckit.analyze` - 仕様、計画、タスク間の整合性を検証
- `/speckit.checklist` - 要件の網羅性と明確性を検証

### Spec Kit ワークフロー

1. `/speckit.constitution` でプロジェクトの基礎を確立
2. `/speckit.specify` で構築したいものを定義
3. `/speckit.plan` で技術アーキテクチャを作成
4. `/speckit.tasks` でタスクを生成
5. `/speckit.implement` で実装を実行

詳細は [Spec Kit ドキュメント](https://github.com/akiojin/spec-kit) を参照してください。

## プロジェクト構造

```
@akiojin/claude-worktree/
├── src/
│   ├── index.ts          # メインアプリケーションエントリーポイント
│   ├── git.ts           # Git操作とブランチ管理
│   ├── worktree.ts      # ワークツリー作成と管理
│   ├── claude.ts        # Claude Code 統合
│   ├── codex.ts         # Codex CLI 統合
│   ├── github.ts        # GitHub CLI統合
│   ├── utils.ts         # ユーティリティ関数とエラーハンドリング
│   └── ui/              # ユーザーインターフェースコンポーネント
│       ├── display.ts   # コンソール出力フォーマット
│       ├── prompts.ts   # 対話型プロンプト
│       ├── table.ts     # ブランチテーブル生成
│       └── types.ts     # TypeScript型定義
├── bin/
│   └── claude-worktree.js # 実行可能ラッパー
├── .claude/             # Claude Code 設定
│   └── commands/        # Spec Kit スラッシュコマンド
├── .specify/            # Spec Kit スクリプトとテンプレート
│   ├── memory/          # プロジェクトメモリファイル
│   ├── scripts/         # 自動化スクリプト
│   └── templates/       # 仕様書テンプレート
├── specs/               # 機能仕様書
└── dist/                # コンパイル済みJavaScript出力
```

## 開発

### セットアップ

```bash
# リポジトリをクローン
git clone https://github.com/akiojin/claude-worktree.git
cd claude-worktree

# 依存関係をインストール（bun）
bun install

# プロジェクトをビルド（bun）
bun run build
```

### 利用可能なスクリプト

```bash
# 自動リビルド付き開発モード（bun）
bun run dev

# プロダクションビルド（bun）
bun run build

# 型チェック（bun）
bun run type-check

# コードリンティング（bun）
bun run lint

# ビルド成果物をクリーン（bun）
bun run clean

# CLIをローカルテスト（bun）
bun run start
```

### 開発ワークフロー

1. **フォークとクローン**: リポジトリをフォークし、あなたのフォークをクローン
2. **ブランチ作成**: ツール自体を使用してfeatureブランチを作成
3. **開発**: TypeScriptサポート付きで変更を実施
4. **テスト**: `bun run start`でCLI機能をテスト
5. **品質チェック**: `bun run type-check`と`bun run lint`を実行
6. **プルリクエスト**: 明確な説明付きでPRを提出

### コード構造

- **エントリーポイント**: `src/index.ts` - メインアプリケーションロジック
- **コアモジュール**: Git操作、ワークツリー管理、Claude統合
- **UIコンポーネント**: `src/ui/`のモジュラーインターフェースコンポーネント
- **型安全性**: 包括的なTypeScript定義
- **エラーハンドリング**: 全モジュールにわたる堅牢なエラー管理

## 統合例

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
**Bunバージョン**: `bun --version`でBun >= 1.0.0を確認

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

### アイコン凡例

- 先頭3枠: ⚡(main/develop) / ✨(feature) / 🔥(hotfix) / 📦(release) / 📌(other)、🟢=ワークツリーあり、🟠=ワークツリーあり(アクセス不可)、✏️=未コミット、⚠️=警告、⭐=現在ブランチ
- 配置枠: 空白=ローカルが存在、`☁`=リモートのみ存在
- 選択枠: カラー環境では背景反転を用いず `>`プレフィックス（スペース付き）で選択中を表示
