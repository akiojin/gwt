# gwt

gwt は Git worktree の管理と、`Claude Code` / `Codex` / `Gemini` /
`OpenCode` などのコーディングエージェント起動を行うデスクトップ GUI です。

## インストール

[GitHub Releases](https://github.com/akiojin/gwt/releases) からお使いの
プラットフォーム向けバイナリをダウンロードし、`PATH` に配置してください。

### macOS

```bash
curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/install.sh | bash
```

特定バージョンを指定する場合:

```bash
curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/install.sh | bash -s -- --version 6.30.3
```

### Windows / Linux

GitHub Releases からバイナリを取得して `PATH` に配置します。

### アンインストール（macOS）

```bash
curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/uninstall.sh | bash
```

## 前提

- `PATH` 上で `git` が使えること
- GitHub 連携機能を使う場合は `gh auth login` 済みであること
- エージェント利用時は必要な API キーを設定すること
  - `ANTHROPIC_API_KEY` または `ANTHROPIC_AUTH_TOKEN`
  - `OPENAI_API_KEY`
  - `GOOGLE_API_KEY` または `GEMINI_API_KEY`
- shared project index runtime の bootstrap / repair が必要な場合は
  Python 3.9+ が使えること

Linux デスクトップ版のビルドには WebKitGTK 系の依存が必要です。CI と同じ依存は
[docs/docker-usage.md](docs/docker-usage.md) を参照してください。

## 使い方

ネイティブ GUI を起動します。

```bash
gwt
```

起動時には前回の状態を復元するか、新しいプロジェクトディレクトリを開けます。
同時に WebView 用のローカル HTTP/WebSocket サーバーも起動し、stderr に
`http://127.0.0.1:<port>/` のような URL を出力します。ネイティブアプリの起動中は、
同じ URL を通常のブラウザでも開けます。

CLI サブコマンドは同じ `gwt` バイナリで処理され、GUI は起動しません。

```bash
gwt issue spec 1784 --section plan
gwt pr current
gwt board show
gwt hook workflow-policy
```

## 基本フロー

1. リポジトリを開く、または前回のプロジェクトを復元する
2. キャンバス上に必要なウィンドウを並べる
3. `Branches` でブランチを選択し、ダブルクリックで Launch Agent を開く
4. `Shell` または `Agent` ウィンドウを起動する
5. `File Tree` でリポジトリを閲覧する

利用できるウィンドウ:

- `Shell`
- `Agent`
- `Branches`
- `File Tree`
- `Settings`
- `Memo`
- `Profile`
- `Logs`
- `Issue`
- `SPEC`
- `Board`
- `PR`

`Shell` と `Agent` は実プロセスを持つウィンドウです。`File Tree` は実装済みの
read-only ツリーです。それ以外は現時点では mock surface です。

## キャンバス操作

- 画面上の zoom ボタンでキャンバスを拡大・縮小
- 背景ドラッグでキャンバスを移動
- `Tile` で表示中のウィンドウをグリッド整列
- `Stack` でタイトルバーを残したまま重ねて表示
- `Cmd/Ctrl+Shift+Right` と `Cmd/Ctrl+Shift+Left` でフォーカス切替
  - フォーカスされたウィンドウは中央へ寄ります

## Managed Hook と SPEC キャッシュ

- gwt は `.claude/settings.local.json` を再生成し、`.codex/hooks.json` をマージします
- SPEC は `gwt-spec` ラベル付き GitHub Issue として管理されます
- ローカルキャッシュ:
  `~/.gwt/cache/issues/<repo-hash>/`
- SPEC 全体を読む:

```bash
gwt issue spec <number>
```

- セクション単位で読む:

```bash
gwt issue spec <number> --section spec|plan|tasks
```

## ログ

- アプリログ:
  `~/.gwt/logs/<repo-hash>/gwt.log.YYYY-MM-DD`
- GUI 状態:
  `~/.gwt/workspace-state.json`

## 開発

### ビルド

```bash
cargo build -p gwt
```

### 実行

```bash
cargo run -p gwt
```

### macOS 向け `.app` bundle

```bash
cargo install cargo-bundle
cargo bundle -p gwt --format osx
```

### テスト

```bash
cargo test -p gwt-core -p gwt --all-features
```

### Lint

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

### フォーマット

```bash
cargo fmt
```

## プロジェクト構成

```text
├── Cargo.toml          # ワークスペース設定
├── crates/
│   ├── gwt/            # Desktop GUI + WebView server + CLI dispatch
│   ├── gwt-core/       # コアライブラリ
│   └── gwt-github/     # GitHub Issue SPEC cache / update layer
└── package.json        # npm package metadata and postinstall
```

## SPEC

詳細仕様は `gwt-spec` ラベル付き GitHub Issue にあります。ローカルキャッシュ経由で
`gwt issue spec <n>` を使って確認できます。

## ライセンス

MIT
