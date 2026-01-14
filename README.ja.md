# @akiojin/gwt

[English](README.md)

Claude Code / Codex CLI / Gemini CLI 対応の対話型Gitワークツリーマネージャー（グラフィカルなブランチ選択と高度なワークフロー管理機能付き）

## 概要

`@akiojin/gwt`は、直感的なインターフェースを通じてGitワークツリー管理を革新する強力なCLIツールです。Claude Code / Codex CLI / Gemini CLI の開発ワークフローとシームレスに統合し、インテリジェントなブランチ選択、自動ワークツリー作成、包括的なプロジェクト管理機能を提供します。

## 移行ステータス

Rust版はCLI/TUIの主要フローとWeb UI（REST + WebSocket端末）まで実装済みです。残作業はGitバックエンドのフォールバック範囲の整理、ドキュメント整備、リリース周りの調整に集中しています。

## 主要機能

- **モダンTUI**: Ratatuiによるスムーズでレスポンシブなターミナルインターフェース
- **フルスクリーンレイアウト**: 統計情報付きの固定ヘッダー、スクロール可能なブランチリスト、キーボードショートカット付きの常時表示フッター
- **スマートブランチ作成**: ガイド付きプロンプトと自動ベースブランチ選択でfeature、bugfix、hotfix、releaseブランチを作成
- **高度なワークツリー管理**: 作成、クリーンアップ、パス最適化を含む完全なライフサイクル管理
- **Coding Agent 選択**: 起動時の対話型ランチャーで Claude Code / Codex CLI / Gemini CLI を選択
- **Coding Agent 統合**: 選択したコーディングエージェントをワークツリーで起動（Claude Codeは権限設定・変更処理の統合あり）
- **GitHub PR統合**: マージされたプルリクエストのブランチとワークツリーの自動クリーンアップ
- **変更管理**: 開発セッション後のコミット、stash、破棄の内蔵サポート
- **ユニバーサルパッケージ**: 一度インストールすれば全プロジェクトで一貫した動作
- **リアルタイム統計**: 自動ターミナルリサイズ対応でブランチ・ワークツリー数をライブ更新

## インストール

### GitHub Releasesから（推奨）

[Releasesページ](https://github.com/akiojin/gwt/releases)からプリビルドバイナリをダウンロード:

- `gwt-linux-x86_64` - Linux x86_64
- `gwt-linux-aarch64` - Linux ARM64
- `gwt-macos-x86_64` - macOS Intel
- `gwt-macos-aarch64` - macOS Apple Silicon
- `gwt-windows-x86_64.exe` - Windows x86_64

```bash
# Linux x86_64の例
curl -L https://github.com/akiojin/gwt/releases/latest/download/gwt-linux-x86_64 -o gwt
chmod +x gwt
sudo mv gwt /usr/local/bin/
```

### npm/bunx経由

グローバルインストールまたはインストールなしで実行:

```bash
# グローバルインストール
npm install -g @akiojin/gwt
bun add -g @akiojin/gwt

# 一回限りの実行
npx @akiojin/gwt
bunx @akiojin/gwt
```

### ソースからビルド

```bash
# リポジトリをクローン
git clone https://github.com/akiojin/gwt.git
cd gwt

# リリースバイナリをビルド
cargo build --release

# バイナリは target/release/gwt にあります
./target/release/gwt
```

## クイックスタート

任意のGitリポジトリで実行:

```bash
# グローバルインストール済みまたはPATHに追加済みの場合
gwt

# または一回限りの実行にbunxを使用
bunx @akiojin/gwt
```

CLIオプション:

```bash
# ヘルプを表示
gwt --help

# バージョンを確認
gwt --version

# ワークツリー一覧
gwt list

# 既存ブランチ用のワークツリーを追加
gwt add feature/my-feature

# 新規ブランチとワークツリーを作成
gwt add -n feature/new-feature --base develop

# ワークツリーを削除
gwt remove feature/old-feature

# 孤立したワークツリーをクリーンアップ
gwt clean
```

ツールは以下のオプションを持つ対話型インターフェースを提供します:

1. **既存ブランチを選択**: ワークツリー自動作成機能付きでローカル・リモートブランチから選択
2. **新規ブランチ作成**: タイプ選択（feature/bugfix/hotfix/release）によるガイド付きブランチ作成
3. **ワークツリー管理**: 既存ワークツリーの表示、オープン、削除
4. **ブランチクリーンアップ**: マージ済みPRやベースブランチと差分がないブランチ／ワークツリーをローカルから自動削除

## 高度なワークフロー

### ブランチ戦略

このリポジトリは構造化されたブランチ戦略に従います：

- **`main`**: 本番環境用コード。リリース専用の保護ブランチ。
- **`develop`**: 機能統合ブランチ。すべてのfeatureブランチはここにマージ。
- **`feature/*`**: 新機能と機能強化。**`develop`をベースとし、`develop`をターゲットにする必要があります**。
- **`hotfix/*`**: 本番環境の緊急修正。`main`をベースとし、ターゲットにする。
- **`release/*`**: リリース準備ブランチ。

**重要**: featureブランチを作成する際は、常に`develop`をベースブランチとして使用してください：

```bash
# 正しい方法: develop からfeatureブランチを作成
git checkout develop
git pull origin develop
git checkout -b feature/my-feature

# またはこのツールを使用すると自動的に処理されます
gwt
# → 「新規ブランチ作成」を選択 → 「feature」→ 自動的にdevelopをベースとして使用
```

### ブランチ作成ワークフロー

> **重要**: このワークフローは人間の開発者向けです。エージェントは、ユーザーから明確かつタスク固有の指示がない限り、ブランチの作成や削除を絶対に行ってはいけません。

1. メインメニューから「新規ブランチ作成」を選択
2. ブランチタイプ（feature、bugfix、hotfix、release）を選択
3. 自動プレフィックス適用でブランチ名を入力
4. 利用可能なオプションからベースブランチを選択
5. ワークツリー作成パスを確認
6. 自動ワークツリーセットアップと選択ツールの起動

### ワークツリー管理

- **既存を開く**: 既存ワークツリーで選択ツールを起動
- **ワークツリー削除**: オプションのブランチ削除付きクリーン削除
- **バッチ操作**: 複数ワークツリーの効率的な処理

### GitHub統合

- **ブランチクリーンアップ**: マージ済みPRやベースブランチと差分がないブランチを自動検出して安全に削除
- **認証確認**: 操作前にGitHub CLIセットアップを検証
- **リモート同期**: クリーンアップ操作前に最新変更を取得

## システム要件

- **Rust**: Stableツールチェーン（ソースからビルドする場合）
- **Git**: ワークツリーサポート付き最新版
- **Coding Agent**: 少なくともいずれかが必要（Claude Code、Codex CLI、または Gemini CLI）
- **GitHub CLI**: PR クリーンアップ機能に必要（オプション）
- **bun/npm**: bunx/npx実行方式に必要

## プロジェクト構造

```text
@akiojin/gwt/
├── Cargo.toml           # ワークスペース設定
├── crates/
│   ├── gwt-cli/         # CLIエントリポイントとTUI（Ratatui）
│   ├── gwt-core/        # コアライブラリ（ワークツリー管理）
│   ├── gwt-web/         # Webサーバー（将来）
│   └── gwt-frontend/    # Webフロントエンド（将来）
├── package.json         # npm配布用ラッパー
├── bin/gwt.js           # バイナリラッパースクリプト
├── scripts/postinstall.js  # バイナリダウンロードスクリプト
├── specs/               # 機能仕様書
└── docs/                # ドキュメント
```

## 開発

### セットアップ

```bash
# リポジトリをクローン
git clone https://github.com/akiojin/gwt.git
cd gwt

# プロジェクトをビルド
cargo build

# テストを実行
cargo test

# デバッグ出力付きで実行
cargo run
```

### 利用可能なコマンド

```bash
# 開発ビルド
cargo build

# リリースビルド
cargo build --release

# テスト実行
cargo test

# clippy lint実行
cargo clippy --all-targets --all-features -- -D warnings

# コードフォーマット
cargo fmt

# CLIをローカル実行
cargo run
```

### 開発ワークフロー

1. **フォークとクローン**: リポジトリをフォークし、あなたのフォークをクローン
2. **ブランチ作成**: ツール自体を使用してfeatureブランチを作成
3. **開発**: Rustで変更を実施
4. **テスト**: `cargo run`でCLI機能をテスト
5. **品質チェック**: `cargo clippy`と`cargo fmt --check`を実行
6. **プルリクエスト**: 明確な説明付きでPRを提出

### コード構造

- **エントリーポイント**: `crates/gwt-cli/src/main.rs` - メインアプリケーションロジック
- **コアモジュール**: Git操作、ワークツリー管理は`gwt-core`に
- **TUIコンポーネント**: `gwt-cli/src/tui/`のRatatuiベースインターフェース
- **型安全性**: 包括的なRust型定義
- **エラーハンドリング**: `thiserror`による堅牢なエラー管理

## リリースプロセス

利用者の方は GitHub Releases もしくは npm で公開される最新版をご利用ください。リリース自動化の概要は [docs/release-guide.md](./docs/release-guide.md)（日本語版: [docs/release-guide.ja.md](./docs/release-guide.ja.md)）にまとめています。

## トラブルシューティング

### よくある問題

**権限エラー**: 適切なディレクトリ権限があることを確認
**Git ワークツリー競合**: クリーンアップ機能を使用して古いワークツリーを削除
**GitHub認証**: PRクリーンアップ機能使用前に`gh auth login`を実行
**バイナリが見つからない**: gwtバイナリがPATHに含まれていることを確認

### デバッグモード

詳細出力には環境変数を設定:

```bash
GWT_DEBUG=1 gwt
```

## ライセンス

MIT - 詳細はLICENSEファイルを参照

## 貢献

貢献を歓迎します！以下の貢献ガイドラインをお読みください:

1. **Issues**: GitHub IssuesでバグレポートやFeatureリクエストを報告
2. **プルリクエスト**: 上記の開発ワークフローに従う
3. **コードスタイル**: Rustベストプラクティスと既存パターンを維持
4. **ドキュメント**: 重要な変更についてはREADMEとコードコメントを更新

### 貢献者

- AI Novel Project Team
- コミュニティ貢献者歓迎

## サポート

- **ドキュメント**: このREADMEとインラインコードドキュメント
- **Issues**: バグレポートとFeatureリクエスト用のGitHub Issues
- **ディスカッション**: 質問とコミュニティサポート用のGitHub Discussions
