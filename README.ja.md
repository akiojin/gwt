# gwt

gwt は agent-driven development のためのデスクトップ control plane です。
コーディングエージェント、プロジェクト文脈、共有 coordination、GitHub
Issue ベースの SPEC、セマンティック検索、managed workflow automation を、
ネイティブ GUI とブラウザから開ける 1 つの workspace に集約します。

Git worktree は gwt の背後にある隔離基盤です。gwt は安全な task ごとの
Agent workspace を materialize するために worktree を使いますが、利用者の
主導線は branch 管理ではなく、作業、Issue、SPEC、検索、Board 文脈から始まります。

## gwt の特徴

- **Agent workspace** — `Claude Code` / `Codex` / `Gemini` / `OpenCode` /
  `Copilot` / custom agent を共有 canvas から起動・再開・状態確認できます。
- **Shared Board** — user と agent の communication を repo-scoped timeline に集約し、
  `status` / `claim` / `next` / `blocked` / `handoff` / `decision` /
  `question` を扱えます。
- **Agent 間 coordination** — managed hooks が reasoning milestone の投稿を促し、
  直近の Board 文脈を注入するため、並列 Agent が判断・引き継ぎ・ブロッカー・
  自分宛 request を把握できます。
- **Semantic Knowledge Bridge** — Issue、SPEC、project source、docs を
  substring だけでなく ChromaDB / multilingual-e5 の semantic index で検索できます。
- **GitHub Issue-backed SPEC** — `gwt-spec` Issue を正本にしつつ、
  ローカル cache-backed CLI で section 単位に読み書きできます。
- **Managed workflow skills** — discussion、Issue routing、planning、
  TDD implementation、PR、architecture review、project search、agent 管理用の
  bundled `gwt-*` skills を使えます。
- **Operator canvas** — Agent、Board、Issue、SPEC、Logs、Memo、Profile、
  File Tree、Branches、PR surface を mission-control 風 workspace に並べられます。

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
gwtd daemon status            # プロジェクトごとの runtime daemon を確認
```

managed hook と runtime 委譲は `gwtd` を使います。macOS と Linux では、
ユーザーが `gwtd daemon start` を実行することでプロジェクトごとの
runtime daemon（Unix ドメインソケット IPC）が起動します。daemon
が live な間、同じリポジトリに繋がっている全 `gwt` インスタンスへ
イベントが fan-out されます（例: 片方のウィンドウで Board に投稿
した内容が、別インスタンスにも遅延なく届く）。Ctrl-C / SIGTERM で
daemon を停止するまでバックグラウンドで動き続けます。診断用に
`gwtd daemon status` で現在の endpoint を確認できます。`gwtd
daemon start` を実行していない場合は multi-instance fan-out は
無効ですが、ローカルのファイルベース state とファイル watcher は
従来どおり動作します。

Windows では現状 long-running daemon は提供されておらず、
`gwtd daemon start` は "not yet implemented" で終了します。managed
hook は同期的な `gwt hook ...` dispatch にフォールバックし、複数
インスタンス間のイベント fan-out は Windows 対応 (named-pipe 経路)
が完了するまで利用できません。`gwtd daemon status` 自体は Windows
でも実行可能ですが、daemon が動かないため常に `stopped` を表示します。

## Agent Workflow

1. プロジェクトを開く、または前回のプロジェクトを復元する
2. `Board`、`Issue`、`SPEC`、Knowledge search surface で現在の作業、
   関連 owner、過去の判断を把握する
3. まだ branch ではなく作業単位として曖昧な場合は、Project Bar または
   Command Palette の `Start Work` を選ぶ
4. `Start Work` から `Agent` を起動する。既知の owner がある場合は
   Issue / SPEC detail から直接 Launch Agent する
5. 起動確定時にのみ、gwt が背後の `work/YYYYMMDD-HHMM[-n]`
   branch / worktree を materialize する
6. Agent 実行中は shared Board に status、claim、next、blocked、
   handoff、decision を残して coordination する
7. Git の確認・filter・cleanup・低レベルな branch/worktree detail が必要な場合だけ
   `Branches` を開く

主なウィンドウ:

- `Agent` — Start Work / Launch Agent から作成される実 coding-agent process window
- `Board` — reasoning と coordination のための user / agent shared timeline
- `Issue` / `SPEC` — semantic search、detail pane、Launch Agent handoff を備えた
  cache-backed Knowledge Bridge
- `Logs` — project diagnostics と live log surface
- `Memo` / `Profile` — repo-scoped note と environment/profile 管理
- `File Tree` — 実リポジトリの read-only tree
- `Branches` — branch 確認、filter、cleanup、Git detail
- `Settings` — application と agent の設定
- `PR` — pull-request workflow surface。詳細な一覧機能は cache-backed PR source の整備に依存します

`Agent` は coding agent セッション用の実プロセスウィンドウです。`Board` は
Agent が status、decision、handoff、request を外部化する coordination surface です。
`Issue` と `SPEC` は frontend から GitHub API response を直接描画せず、
ローカル cache と semantic index を使います。

Windows の Host 起動では、Launch Agent で Command Prompt、Windows PowerShell、
PowerShell 7 を選択できます。Docker 起動では引き続きコンテナ内のシェルを使います。

ターミナルウィンドウでは、テキストをドラッグ選択してマウスボタンを離すとコピー
できます。Windows / Linux では `Ctrl+Shift+C` でも現在の選択をコピーできます。
`Ctrl+C` は実行中のターミナルプロセス向けの割り込みのままです。

## Knowledge、Search、Managed Skills

gwt は project knowledge を Agent workspace の近くに置きます。

- `gwtd issue spec <n>` は GitHub Issue-backed SPEC をローカル cache から読みます。
- `gwtd issue view <n>` と `gwtd issue comments <n>` は gwt CLI surface から
  cache-backed Issue access を提供します。
- `gwt-search` は共有 ChromaDB runtime を通じて SPEC、Issue、source files、
  docs を検索します。index が無い場合は必要に応じて build され、desktop app は
  管理対象 Python search runtime を修復できます。
- Issue / SPEC の Knowledge Bridge window は cache-backed list/detail と、
  semantic ranking、exact match priority、match percentage を組み合わせます。

Bundled workflow skills は active worktree の `.claude/skills`、
`.claude/commands`、`.codex/skills` に materialize されます。公開 entrypoint は
以下です。

- `gwt-discussion` — investigation-first な議論と設計明確化
- `gwt-register-issue` / `gwt-fix-issue` — Issue intake と Issue 起点の修正
- `gwt-plan-spec` — 承認済み SPEC の implementation planning
- `gwt-build-spec` — 承認済み task からの TDD-oriented implementation
- `gwt-manage-pr` — PR create/check/fix lifecycle
- `gwt-arch-review` — architecture review と改善 routing
- `gwt-search` — unified semantic search
- `gwt-agent` — running agent pane の確認と操作

Managed hooks は user hook を保持しながら、Agent state、workflow guardrails、
Board reminders、discussion/plan/build Stop checks、coordination-event summaries
を追加します。

## ワークスペース基盤

Agent session の隔離と再現性のため、gwt は各プロジェクトをワークスペース
ディレクトリ配下の **Nested Bare + Worktree** 構成として管理できます。

```
<workspace>/<project>/
├── <project>.git/          # bare リポジトリ
├── develop/                # develop ワークツリー（既定の作業ディレクトリ）
├── feature/<name>/         # ブランチごとの追加ワークツリー
└── .gwt/project.toml       # gwt が管理するプロジェクトメタデータ
```

Initialization ウィザード経由で clone した場合は自動でこの構成になります。
既存の Normal Git リポジトリ（プロジェクト直下に `.git/` がある通常レイアウト）
は検出され、要望に応じて Nested Bare + Worktree 構成へマイグレーションできます。
マイグレーションは `.gwt-migration-backup/` にフルバックアップを取ってから
bare リポジトリを作り直し、各 worktree を新レイアウトに再配置します。
任意のフェーズで失敗した場合は自動的に元のレイアウトへロールバックされます。
進行状況は
[GitHub Issue #1934 (SPEC-1934)](https://github.com/akiojin/gwt/issues/1934)
で管理しています。

既存の Normal Git プロジェクトを移行するには、gwt のプロジェクトピッカーまたは
`Reopen Recent` から開きます。gwt がレイアウトを検出すると、3 択の確認モーダル
が表示されます。

- **Migrate** — 即座にマイグレーションを実行。進捗はフェーズ単位で
  ストリーミング (Validate → Backup → Bareify → Worktrees → Submodules →
  Tracking → Cleanup → Done) され、成功時はアプリを再起動せずに新しいブランチ
  worktree にプロジェクトタブが切り替わります。
- **Skip** — Normal Git のままプロジェクトを開きます。次回そのプロジェクトを
  開いたときに再度モーダルが表示されます。
- **Quit** — リポジトリに触れずにアプリを終了します。

## キャンバス操作

- 画面上の zoom ボタンでキャンバスを拡大・縮小
- 背景ドラッグでキャンバスを移動
- `Tile` で表示中のウィンドウをグリッド整列
- `Stack` でタイトルバーを残したまま重ねて表示
- `Cmd/Ctrl+Shift+Right` と `Cmd/Ctrl+Shift+Left` でフォーカス切替
  - フォーカスされたウィンドウは中央へ寄ります

## Operator デザイン言語 (SPEC-2356)

Operator Design System 採用後、gwt は editorial-industrial 系タイポグラフィ
(本文 `Mona Sans` / ディスプレイ `Hubot Sans Condensed` / 等幅 `JetBrains Mono`)
を中核とした単一の mission-control サーフェスとして再設計されました。
Project Bar / Sidebar Layers / Status Strip / Command Palette / Hotkey
Overlay / Drawer モーダル / フローティングウィンドウ の全クロームが
共通トークンを参照し、 2 つの旗艦テーマで提供されます:

- **Dark Operator** (Mission Control / carbon + neon) — 既定、 長時間作業向け
- **Light Operator** (Drafting Table / bone + ink) — 明るい環境向け

OS の `prefers-color-scheme` に追従しつつ、 Project Bar の **Theme** トグルで
`auto → dark → light → auto` を循環できます。 選択はブラウザストレージに
永続化され、 再起動後も維持されます。 xterm 端末パネルは overall theme に
追従します。 `prefers-reduced-motion: reduce` を有効にすると Living Telemetry
の pulse rim・Status Strip の ticking・Mission Briefing intro が静止表現に
縮退します。 `forced-colors: active` (Windows High Contrast / macOS Increase
Contrast) では system colors にフォールバックします。

### ホットキー

| 組合せ | 動作 |
| --- | --- |
| `⌘K` / `⌘P` | Command Palette を開く (全サーフェス アクションの fuzzy 検索) |
| `⌘B` | Board サーフェスを focus |
| `⌘G` | Git (Branches) サーフェスを focus |
| `⌘L` | Logs サーフェスを focus |
| `⌘?` | Hotkey Overlay (cheat sheet) を toggle |
| `Esc` | 開いている Palette / Overlay / Drawer を閉じる |

## SPEC と runtime クイックリファレンス

- SPEC の正本: `gwt-spec` ラベル付き GitHub Issue
- ローカルキャッシュ:
  `~/.gwt/cache/issues/<repo-hash>/`
- Managed agent integration files:
  `.claude/settings.local.json` と `.codex/hooks.json`
- SPEC 一覧を読む:

```bash
gwtd issue spec list
```

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
