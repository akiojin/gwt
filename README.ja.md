# gwt

gwt は Git worktree の管理と、`Claude Code` / `Codex` / `Gemini` /
`OpenCode` などのコーディングエージェント起動を行うデスクトップ GUI です。

## インストール

[GitHub Releases](https://github.com/akiojin/gwt/releases) からお使いの
プラットフォーム向け release asset を取得してください。

### macOS

- GUI 向けの主配布物: `gwt-macos-universal.dmg`
- マウントした DMG から `GWT.app` を開くとネイティブ GUI をそのまま起動できます
- `PATH` に `gwt` / `gwtd` CLI を入れたい場合は install script を使います

```bash
curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/install.sh | bash
```

特定バージョンを指定する場合:

```bash
curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/install.sh | bash -s -- --version <version>
```

### Windows

- GUI 向けの主配布物: `gwt-windows-x86_64.msi`
- portable bundle: `gwt-windows-x86_64.zip`
- public front door は `gwt.exe` で、`gwtd.exe` は内部 runtime 用の companion binary です

### Linux

- portable bundle:
  - `gwt-linux-x86_64.tar.gz`
  - `gwt-linux-aarch64.tar.gz`
- 展開した `gwt` / `gwtd` を `PATH` 上のディレクトリへ配置します

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

CLI サブコマンドは `gwtd` で処理され、GUI は起動しません。

```bash
gwtd issue spec 1784 --section plan
gwtd pr current
gwtd board show
gwtd hook workflow-policy
```

managed hook と runtime 委譲は `gwtd` を使います。利用者が別の daemon
プロセスを手動起動する必要はありません。

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

Windows の Host 起動では、Launch Agent で Command Prompt、Windows PowerShell、
PowerShell 7 を選択できます。選択したシェルは `Shell` と `Agent` の両方に適用され、
Docker 起動では引き続きコンテナ内のシェルを使います。

ターミナルウィンドウでは、テキストをドラッグ選択してマウスボタンを離すとコピー
できます。Windows / Linux では `Ctrl+Shift+C` でも現在の選択をコピーできます。
`Ctrl+C` は実行中のターミナルプロセス向けの割り込みのままです。

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
gwtd issue spec <number>
```

- セクション単位で読む:

```bash
gwtd issue spec <number> --section spec|plan|tasks
```

## ログ

- アプリログ:
  `~/.gwt/projects/<repo-hash>/logs/gwt.log.YYYY-MM-DD`
- セッション状態:
  `~/.gwt/session.json`
- プロジェクト単位のワークスペース状態:
  `~/.gwt/projects/<repo-hash>/workspace.json`

## 開発

### ビルド

```bash
cargo build -p gwt --bin gwt --bin gwtd
```

### 実行

```bash
cargo run -p gwt --bin gwt
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

### Release Asset Contract

```bash
npm run test:release-assets
```

### Frontend Bundle Contract

```bash
npm run test:frontend-bundle
```

### Release Flow Checks

```bash
npm run test:release-flow
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
`gwtd issue spec <n>` を使って確認できます。

## ライセンス

MIT
