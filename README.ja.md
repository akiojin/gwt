# @akiojin/gwt

[English](README.md)

Claude Code / Codex CLI / Gemini CLI / OpenCode 対応の対話型Gitワークツリーマネージャー（グラフィカルなブランチ選択と高度なワークフロー管理機能付き）

## 概要

`@akiojin/gwt`は、直感的なインターフェースを通じてGitワークツリー管理を革新する強力なCLIツールです。Claude Code / Codex CLI / Gemini CLI / OpenCode の開発ワークフローとシームレスに統合し、インテリジェントなブランチ選択、自動ワークツリー作成、包括的なプロジェクト管理機能を提供します。

## 移行ステータス

Rust版はCLI/TUIの主要フローとWeb UI（REST + WebSocket端末）まで実装済みです。残作業はGitバックエンドのフォールバック範囲の整理、ドキュメント整備、リリース周りの調整に集中しています。

## 主要機能

- **モダンTUI**: Ratatuiによるスムーズでレスポンシブなターミナルインターフェース
- **フルスクリーンレイアウト**: リポジトリ情報付きの固定ヘッダー、枠線付きのブランチリスト
- **ブランチサマリーパネル**: コミット履歴、変更統計、ブランチメタデータに加えて、Tabでセッション要約を切り替えて表示
- **スマートブランチ作成**: ガイド付きプロンプトと自動ベースブランチ選択でfeature、bugfix、hotfix、releaseブランチを作成
- **高度なワークツリー管理**: 作成、Worktreeのあるブランチのクリーンアップ、パス最適化を含む完全なライフサイクル管理
- **Coding Agent 選択**: 起動時の対話型ランチャーでビルトイン（Claude Code / Codex CLI / Gemini CLI / OpenCode）または `~/.gwt/tools.json` 定義のカスタムを選択
- **Coding Agent 統合**: 選択したコーディングエージェントをワークツリーで起動（Claude Codeは権限設定・変更処理の統合あり）
- **GitHub PR統合**: マージされたプルリクエストのブランチとワークツリーの自動クリーンアップ
- **変更管理**: 開発セッション後のコミット、stash、破棄の内蔵サポート
- **tmux マルチエージェントモード**: tmux ペインを使用して複数のコーディングエージェントを並列実行（tmux 内で実行時に自動有効化）
- **ユニバーサルパッケージ**: 一度インストールすれば全プロジェクトで一貫した動作
- **エージェントモード**: 自然言語で機能要求を入力すると、仕様策定・タスク分割・並列実行・PR作成まで自律的に実行

## エージェントモード

エージェントモードは、自然言語の機能要求からコード実装までを自律的に行うマスター/サブエージェントアーキテクチャです。

### 基本操作

1. tmuxセッション内で`gwt`を起動
2. `Tab`キーでエージェントモードに切り替え
3. チャット入力欄に機能要求を入力（例: "ユーザー認証機能を追加"）
4. マスターエージェントがSpec Kitワークフロー（仕様策定→計画→タスク分割）を自動実行
5. 計画を確認し、承認
6. サブエージェントが各タスクを並列実行
7. テスト検証・PR作成まで自動で完了

### 主要機能

- **Spec Kit統合**: 仕様書・計画書・タスクリストを自動生成
- **並列実行**: 依存関係を考慮してタスクを並列実行
- **セッション永続化**: 中断しても`~/.gwt/sessions/`に保存され、再開可能
- **ドライランモード**: "dry run"または"計画だけ"と入力すると計画のみ生成
- **コンテキスト圧縮**: 長い会話を自動要約してトークン消費を削減
- **定期進捗報告**: 2分間隔で実行中タスクの状態を報告

### キーバインド（エージェントモード）

| キー | 動作 |
|-----|------|
| `Tab` | ブランチリスト ↔ エージェントモード切り替え |
| `Enter` | メッセージ送信 / 計画承認 |
| `Esc` | 実行中タスクを一時停止 |
| `Shift+S` | Spec Kitウィザードを開く（ブランチリスト画面） |

### 設計ドキュメント

- [仕様書](specs/SPEC-ba3f610c/spec.md)
- [実装計画](specs/SPEC-ba3f610c/plan.md)
- [クイックスタート](specs/SPEC-ba3f610c/quickstart.md)

## インストール

GitHub Releases を正とし、npm/bunx では該当リリースのバイナリをダウンロードして実行します。

### GitHub Releasesから（推奨）

[Releasesページ](https://github.com/akiojin/gwt/releases)からプリビルドバイナリをダウンロード。各リリースには全対応プラットフォームのバイナリが含まれます:

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

### Cargo 経由

Cargo で CLI を直接インストールできます:

```bash
# cargo-binstall でインストール（高速、GitHub Releasesからプリビルドバイナリをダウンロード）
cargo binstall gwt-cli

# GitHub からインストール（最新開発版）
cargo install --git https://github.com/akiojin/gwt --package gwt-cli --bin gwt --locked

# ローカルチェックアウトからインストール
cargo install --path crates/gwt-cli

# そのまま実行
cargo run -p gwt-cli
```

### ソースからビルド

```bash
# リポジトリをクローン
git clone https://github.com/akiojin/gwt.git
cd gwt

# リリースバイナリをビルド（デフォルト: gwt-cli）
cargo build --release

# ワークスペース全体をビルド（Web/wasm含む）
cargo build --workspace

# バイナリは target/release/gwt にあります
./target/release/gwt
```

Note: CI の Node ベースツール（例: commitlint）は Corepack 経由の pnpm で実行します。ロックファイルは `pnpm-lock.yaml` を正とし、`package-lock.json` は使用しません。

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

# ログを表示
gwt logs --limit 100

# ログをフォロー
gwt logs --follow
```

ツールは以下のオプションを持つ対話型インターフェースを提供します:

1. **既存ブランチを選択**: ワークツリー自動作成機能付きでローカル・リモートブランチから選択
2. **新規ブランチ作成**: タイプ選択（feature/bugfix/hotfix/release）によるガイド付きブランチ作成
3. **ワークツリー管理**: 既存ワークツリーの表示、オープン、削除
4. **ブランチクリーンアップ**: マージ済みPRやベースブランチと差分がないブランチ／ワークツリーをローカルから自動削除（Worktreeのないブランチは対象外）

## キーボードショートカット

### ブランチリスト画面

| キー | 動作 |
|-----|------|
| `Enter` | 既存エージェントペインにフォーカス / 非表示ペインを表示 / ウィザードを開く |
| `d` | エージェントペインを削除（確認あり） |
| `v` | GitViewを開く（選択中ブランチのgit状態詳細） |
| `Space` | ブランチの選択/選択解除 |
| `Up/Down` | ブランチ間を移動 |
| `PageUp/PageDown` | ページ移動 |
| `Home/End` | 先頭/末尾のブランチへジャンプ |
| `f` | フィルターモードに入る |
| `r` | ブランチリストを更新 |
| `c` | マージ済みブランチのクリーンアップ |
| `l` | ログを表示 |
| `?` | ヘルプ |
| `q` / `Ctrl+C` | 終了 |

マウス:
- ブランチ行をダブルクリックするとEnter相当の操作が実行されます（ペインフォーカス/ウィザード起動）。

### フィルターモード

| キー | 動作 |
|-----|------|
| `Esc` | フィルターモードを終了 |
| 入力 | ブランチ名でフィルター |

### GitView画面

GitView画面は、選択中ブランチの詳細なgit状態（ファイル一覧、直近コミット）を表示します。

| キー | 動作 |
|-----|------|
| `Up/Down` | ファイル・コミット間を移動 |
| `Space` | ファイルのdiffまたはコミット詳細を展開/折りたたみ |
| `Enter` | PRリンクをブラウザで開く（ヘッダーにフォーカス時） |
| `v` / `Esc` | ブランチリストに戻る |

マウス:
- ヘッダーのPRリンクをクリックするとブラウザで開きます。

## ステータスアイコンの凡例

| アイコン | 色 | 意味 |
|---------|-----|------|
| `o` | 緑 | 安全 - コミットされていない変更やプッシュされていないコミットなし |
| `!` | 赤 | 未コミット - ローカルに変更あり |
| `^` | 黄 | 未プッシュ - リモートにプッシュされていないコミットあり |
| `*` | 黄 | 未マージ - マージされていない変更あり |

## エージェントステータス表示

ブランチリストの右側に、実行中のエージェントが表示されます:

| 形式 | 意味 |
|------|------|
| `[/] Claude 01:23:45` | 実行中のエージェント（スピナー、名前、稼働時間） |
| `[BG] Claude 01:23:45` | 非表示（バックグラウンド）のエージェント（グレーアウト） |

## コーディングエージェント

gwt は PATH 上のエージェントを検出し、ランチャーに表示します。

対応エージェント（ビルトイン）:

- Claude Code (`claude`)
- Codex CLI (`codex`)
- Gemini CLI (`gemini`)
- OpenCode (`opencode`)

### カスタムコーディングエージェント

カスタムエージェントは `~/.gwt/tools.json` に定義するとランチャーに表示されます。

最小例:

```json
{
  "version": "1.0.0",
  "customCodingAgents": [
    {
      "id": "aider",
      "displayName": "Aider",
      "type": "command",
      "command": "aider",
      "defaultArgs": ["--no-git"],
      "modeArgs": {
        "normal": [],
        "continue": ["--resume"],
        "resume": ["--resume"]
      },
      "permissionSkipArgs": ["--yes"],
      "env": {
        "OPENAI_API_KEY": "sk-..."
      },
      "models": [
        { "id": "gpt-4o", "label": "GPT-4o" },
        { "id": "claude-3-opus", "label": "Claude 3 Opus" }
      ],
      "versionCommand": "aider --version"
    }
  ]
}
```

補足:

- `type` は `path` / `bunx` / `command` を指定します。
- `modeArgs` で実行モード別の引数を定義します（Normal/Continue/Resume）。
- `env` はエージェントごとの環境変数（任意）です。
- `models` は任意です。定義するとモデル選択ステップが表示されます。
- `versionCommand` は任意です。定義するとバージョン検出に使用されます。

## Bareリポジトリワークフロー

gwtは効率的なワークツリー管理のためにbareリポジトリワークフローをサポートしています。このアプローチではbareリポジトリ（`.git`データ）をワークツリーから分離し、より整理されたプロジェクト構成を提供します。

### ディレクトリ構造

```text
/project/
├── repo.git/           # Bareリポジトリ
├── main/               # ワークツリー（mainブランチ）
├── feature-x/          # ワークツリー（feature/xブランチ）
└── .gwt/               # gwt設定
    └── project.json
```

### Bareリポジトリのセットアップ

```bash
# bareリポジトリとしてクローン
git clone --bare https://github.com/user/repo.git repo.git

# bareリポジトリからワークツリーを作成
cd repo.git
git worktree add ../main main
git worktree add ../feature-x feature/x
```

### Bareリポジトリでのgwt使用

bareリポジトリまたはそのワークツリー内でgwtを実行した場合:

| 起動場所 | ヘッダー表示 |
|----------|-------------|
| 通常リポジトリ | `Working Directory: /path [branch]` |
| Bareリポジトリ | `Working Directory: /path/repo.git [bare]` |
| ワークツリー（通常） | `Working Directory: /path [branch]` |
| ワークツリー（bare方式） | `Working Directory: /path [branch] (repo.git)` |

### `.worktrees/`方式からのマイグレーション

既存の`.worktrees/`ディレクトリ方式を使用しているリポジトリがある場合、gwtはこれを検出してbareリポジトリ方式へのマイグレーションを提案します:

1. **バックアップ**: `.gwt-migration-backup/`にバックアップを作成
2. **bareリポジトリ作成**: `{repo-name}.git`を作成
3. **ワークツリー移行**: 既存ワークツリーを新構造に移動
4. **クリーンアップ**: 古い`.worktrees/`ディレクトリを削除
5. **設定作成**: `.gwt/project.json`を作成

### サブモジュールサポート

ワークツリー作成時、gwtはサブモジュールが存在する場合は自動的に初期化します。これにより、ワークツリー作成直後からサブモジュールを使用できます。

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
- **Coding Agent**: 少なくともビルトインまたはカスタムのいずれかが必要
- **GitHub CLI**: PR クリーンアップ機能に必要（オプション）
- **bun/npm**: bunx/npx実行方式に必要

## プロジェクト構造

```text
@akiojin/gwt/
├── Cargo.toml           # ワークスペース設定
├── crates/
│   ├── gwt-cli/         # CLIエントリポイントとTUI（Ratatui）
│   ├── gwt-core/        # コアライブラリ（ワークツリー管理）
│   ├── gwt-web/         # Webサーバー（Axum）
│   └── gwt-frontend/    # Webフロントエンド（Leptos CSR）
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

利用者の方は GitHub Releases もしくは npm で公開される最新版をご利用ください。メンテナ向けのリリースフロー要件は `specs/SPEC-77b1bc70/spec.md` を参照してください。

## トラブルシューティング

### よくある問題

**権限エラー**: 適切なディレクトリ権限があることを確認
**Git ワークツリー競合**: クリーンアップ機能を使用して古いワークツリーを削除
**GitHub認証**: PRクリーンアップ機能使用前に`gh auth login`を実行
**バイナリが見つからない**: gwtバイナリがPATHに含まれていることを確認
**Docker + tmux でのUnicode文字化け**: Dockerコンテナ内のtmuxでUnicode文字（Claude Codeのロゴなど）がアンダースコアに化ける場合、tmuxをUTF-8モードで起動してください:

```bash
tmux -u
```

または `~/.tmux.conf` に以下を追加:

```
set -gq utf8 on
```

Dockerコンテナ内でロケールのインストールと設定が必要な場合もあります:

```bash
apt-get update && apt-get install -y locales
sed -i '/en_US.UTF-8/s/^# //g' /etc/locale.gen
locale-gen
export LANG=en_US.UTF-8
export LC_ALL=en_US.UTF-8
```

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
