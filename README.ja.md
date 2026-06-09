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
- **Operator canvas** — Agent、Board、Issue、SPEC、Logs、Profile、
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
- MSI をダブルクリックしても何も起きないように見える場合は、PowerShell から
  診断スクリプトを実行し、生成された出力ディレクトリを Issue 報告に添付してください。

```powershell
$diag = "$env:TEMP\diagnose-windows-msi.ps1"
Invoke-WebRequest `
  https://raw.githubusercontent.com/akiojin/gwt/main/scripts/diagnose-windows-msi.ps1 `
  -OutFile $diag
powershell -ExecutionPolicy Bypass -File $diag `
  -MsiPath "$env:USERPROFILE\Downloads\gwt-windows-x86_64.msi"
```

このスクリプトは MSI の SHA256、Authenticode 署名、Zone.Identifier
download marker、Windows Installer の `msiexec` verbose log、インストール後の
ファイル配置、基本的な `gwt.exe` 起動証跡を記録します。

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

`gwt` を起動するとタスクトレイ (macOS は menubar、Windows は notification
area、Linux は StatusNotifierItem 対応 DE のシステムトレイ) にアイコンが
常駐します。トレイメニューから操作します:

- **Open in browser**: 既定ブラウザで埋込サーバー (`http://127.0.0.1:<port>/`)
  を開きます。同じ URL は他のブラウザでも開けます。
- **Copy URL**: 起動中の tray プロセスの URL を OS clipboard にコピーします。
- **About GWT**: 起動中の tray プロセスのブラウザ版 About / Version 画面を
  開きます。
- **Quit**: tray アイコン + 埋込サーバー + PTY 子プロセスを順に停止します。

Autostart は **Settings > System > Launch GWT at login** で切り替えます。
有効にすると `auto-launch` crate 経由で macOS LaunchAgent / Windows HKCU
Run registry / Linux XDG autostart を user scope に登録し、次回 OS ログイン時に
`gwt` が tray-resident process として起動します。ブラウザは自動では開きません。

```bash
gwt                                 # トレイ常駐 + 埋込サーバー起動 (loopback)
gwt --bind 0.0.0.0 --port 60745     # 埋込サーバーを LAN / VPN 到達可能な IP/Port に bind
gwt open                            # 起動中の tray インスタンスの URL を既定ブラウザで開く
```

`--bind <ip>` の既定値は `127.0.0.1`、`--port <n>` の既定値は `0` (ephemeral) です。
同一 LAN や VPN-extended LAN の別端末からブラウザ UI に接続したい場合は `--bind 0.0.0.0`
を指定してください。ポートを固定したい場合は `--port` で明示できます。`--no-tray` /
`--no-open` は SPEC #2920 Phase 4 の他作業が完了するまで受け取るだけで no-op の状態です。

`gwt open` は Linux の GNOME 3.26+ など system tray を持たない環境向けの
fallback です。tray アイコンが見えない場合でも `gwt browser URL: ...` が
stderr に出力されるので、その URL を手動で開くか `gwt open` を実行して
ください。

タスクトレイ常駐は 1 OS-login ユーザーあたり 1 インスタンスです。同じ
ユーザーで二重に `gwt` を起動した場合、後続プロセスは既存インスタンスの
URL を stderr に出して exit 0 で終了します。

### `gwt serve` 廃止について

`gwt serve` / `gwt --headless` 経路は v10.0.0 で削除されました (SPEC #2920 Q9)。
従来のコマンドを使っていた CI / 自動化スクリプトは、現状の `gwt` 出力
(`gwt browser URL: ...`) と `GWT_BROWSER_URL_FILE` 環境変数のハンドオフ契約を
利用してください。埋込サーバーが起動すると、同じ URL 取得経路を使えます。

信頼境界は **LAN のみ** (VPN-extended LAN を含む) です。埋込ブラウザサーバー
には TLS 終端、認証ゲート、レート制限は組み込まれていません。`--bind` で
公開した IP に到達できる主体はすべて trusted と見なされ、ターミナル起動を含む
全 UI 操作が可能になります。既定値は `127.0.0.1` で、ネイティブ GUI と同じ
ローカルループバック信頼モデルを維持します。外部からアクセスする場合は、
ポートを公開インターネットに晒さず、VPN (Tailscale、WireGuard など) 越しで
LAN に入ってから接続してください。

プラットフォーム注記: Linux では `tao 0.35` が EventLoop 生成時に display
server (X11 / Wayland) を要求します。macOS / Windows のブラウザサーバー起動は
追加の display 設定不要ですが、Linux の pure-headless 環境 (DISPLAY 無し) では
`Xvfb` / `xvfb-run` を併用するか、SPEC-1942 follow-up の tao 切り離し対応を
待ってください。

すべての HTTP / WebSocket リクエストは `tracing::info!(target = "gwt_access",
...)` で記録されるため、stderr と `~/.gwt/logs/<date>/` で「どこからアクセス
されているか」を即時に確認できます。`/healthz` は `debug!` に降格されており、
ヘルスチェックでログが埋まりません。

Lifecycle: 起動中の `gwt` プロセスが agent / PTY の寿命を所有します。ブラウザの
タブを閉じても agent は **停止しません**。`Ctrl-C` / `SIGTERM` を受け取ったときに、
PTY のドレイン → サーバー停止の順で graceful shutdown を実行します。tray 常駐
プロセスは 1 OS-login ユーザーにつき 1 つだけで、二重起動した `gwt` は既存 URL を
出して終了します。

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

1. プロジェクトを開く、GitHub から clone する、または前回のプロジェクトを復元する
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
- `Profile` — environment/profile 管理
- `File Tree` — 実リポジトリの read-only tree
- `Branches` — branch 確認、filter、cleanup、Git detail
- `Settings` — application と agent の設定。`System` タブで Workspace summary
  と Board 投稿本文の出力言語を `Auto / English / 日本語` から選択できます
  (Auto は OS locale を参照し、`C` / `POSIX` や未設定時は English にフォール
  バック)。設定はグローバルで `~/.gwt/config.toml` の `[ai].language` に保存
  されます。UI 文言は引き続き英語固定です (SPEC-1933 NFR-005)。
- `PR` — pull-request workflow surface。詳細な一覧機能は cache-backed PR source の整備に依存します

`Agent` は coding agent セッション用の実プロセスウィンドウです。`Board` は
Agent が status、decision、handoff、request を外部化する coordination surface です。
`Issue` と `SPEC` は frontend から GitHub API response を直接描画せず、
ローカル cache と semantic index を使います。

Windows の Host 起動では、Launch Agent で Command Prompt、Windows PowerShell、
PowerShell 7 を選択できます。Docker 起動では引き続きコンテナ内のシェルを使います。

ターミナルウィンドウでは、テキストをドラッグ選択してマウスボタンを離すとコピー
できます。Windows では `Ctrl+C` で現在の選択をコピーして選択を解除します。
選択がない場合、`Ctrl+C` は実行中のターミナルプロセス向けの割り込みのままです。
Linux では `Ctrl+Shift+C` でも現在の選択をコピーできます。

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

gwt から起動された Agent に live GUI / browser backend がある場合、managed hook
は local hook-forward bridge も有効にします。この bridge は、その session に
gwt が注入した loopback endpoint と bearer token だけへ hook event を POST し、
既存の live event stream 経由で frontend client へ fan-out します。gwt 外から
起動した session には転送先が注入されないため、`gwt hook forward` は silent
no-op のままです。古い転送先、接続拒否、validation error、delivery timeout は
fail-open の診断情報として扱われ、Agent の tool call を block しません。

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

Project Picker の `Clone from GitHub...` (タブ未選択時の全画面ピッカー) と、
トップツールバーの `Open Project ▾` split-button ドロップダウン (アクティブな
プロジェクトがある状態でも到達可能) のどちらからでも clone を開始できます。
clone modal では GitHub HTTPS / SSH URL を直接入力するか、`gh search repos`
による repository 検索から候補を選択し、保存先の親フォルダを指定します。新しい
プロジェクトは `<parent>/<project>/` に作成され、`<project>.git/` bare リポジトリ
と initial worktree が配置されます。remote に `develop` が存在する場合は
`develop` worktree を作成し、存在しない場合は remote default branch を使います。

既存の Normal Git リポジトリ（プロジェクト直下に `.git/` がある通常レイアウト）
は検出され、要望に応じて Nested Bare + Worktree 構成へマイグレーションできます。
マイグレーションは `.gwt-migration-backup/` にフルバックアップを取ってから
bare リポジトリを作り直し、各 worktree を新レイアウトに再配置します。
任意のフェーズで失敗した場合は自動的に元のレイアウトへロールバックされます。
進行状況は
[GitHub Issue #1934 (SPEC-1934)](https://github.com/akiojin/gwt/issues/1934)
で管理しています。

既存の Normal Git プロジェクトを移行するには、gwt のプロジェクトピッカーまたは
`Reopen Recent` から開きます。gwt がレイアウトを検出すると、Migrate 確認
モーダルが表示されます。

**Migrate** を選ぶと即座にマイグレーションを実行します。進捗はフェーズ単位で
ストリーミング (Validate -> Backup -> Bareify -> Worktrees -> Submodules ->
Tracking -> Cleanup -> Done) され、成功時はアプリを再起動せずに新しいブランチ
worktree にプロジェクトタブが切り替わります。

## Board プロバイダ (Local / Slack / Teams)

調整用の **Board** は 3 つのプロバイダのいずれかをバックエンドにできます。
**Settings → System → Board provider** で選択します:

- **Local**（既定）— ファイルシステム保存・オフライン・worktree 単位。設定不要。
- **Slack** — Slack Web API で Slack チャンネルに投稿/読み取り。
- **Teams** — Microsoft Graph 経由の Microsoft Teams チャンネル。*実験的: コードは
  実装済みだが実テナントでの end-to-end 検証は未実施。プレビュー扱い。*

プロバイダを切り替えると Board の内容ごと入れ替わります（各プロバイダは独立した
ストアで、切り替え中は旧プロバイダのエントリは不可視になり、戻すと再表示）。
シークレットや OAuth トークンは `~/.gwt/credentials/` 配下の権限制限ストアに保存し、
`config.toml` には平文で保存しません。

### Slack を Board バックエンドにする

> 📷 *スクショ挿入位置を下記に記載。Slack 管理画面は `api.slack.com`（アカウント
> 固有）、gwt 画面は Settings → System 配下。各ステップでキャプチャを追加。*

#### 1. Slack アプリを作成

1. <https://api.slack.com/apps> → **Create New App** → **From scratch**。
2. 名前（例 `gwt`）と対象ワークスペースを選び **Create App**。
   - 📷 *スクショ: Create App ダイアログ。*

#### 2. リダイレクト URL を追加

1. アプリの **OAuth & Permissions → Redirect URLs → Add New Redirect URL**。
2. gwt の OAuth コールバック URL を**正確に**入力し **Save URLs**:

   ```text
   http://127.0.0.1:8765/oauth/callback
   ```

   - `localhost` ではなく `127.0.0.1`、`/oauth/callback` パスを含め、末尾スラッシュ
     なし。gwt の **OAuth callback port**（既定 `8765`、Settings で変更可 — 手順 5）
     と一致させる必要があります。gwt はポート欄の隣に登録すべき URL を表示します。
   - 📷 *スクショ: 保存済みの Redirect URLs。*

#### 3. Bot スコープを追加

1. **OAuth & Permissions → Scopes → Bot Token Scopes** に追加:
   `chat:write`, `channels:history`, `channels:read`。
2. **Install App → Install to Workspace**（スコープ/リダイレクト変更後は再インストール
   して反映）。
   - 📷 *スクショ: Bot Token Scopes 一覧。*

#### 4. 認証情報を控える

**Basic Information → App Credentials** から **Client ID** と **Client Secret** を控え、
対象チャンネルの **Channel ID**（Slack: チャンネル → **View channel details** →
ダイアログ下部）も控えます。

#### 5. gwt を設定

1. gwt の **Settings → System → Board provider** で **Slack** を選択。
2. フォームに入力し **Save configuration**:
   - **Client ID** / **Default channel ID** / **Client secret**（secret は安全に
     保存され `config.toml` には書かれません。保存後は欄が空になり
     "✓ A client secret is saved" と表示）。
   - 必要なら **OAuth callback port**（既定 `8765`）を変更。フォームに手順 2 で登録
     すべき Redirect URL が表示されます。変更は次回起動で反映。
   - 📷 *スクショ: gwt Settings → System → Board provider = Slack（設定フォーム）。*
3. **Sign in** をクリック → ブラウザで Slack 同意画面 → **Allow（許可）**。
   コールバック画面に "Signed in / Connected the slack Board provider" が表示され、
   gwt が "Signed in to slack" に変わります。
   - 📷 *スクショ: Slack 同意画面と "Signed in" 結果画面。*

#### 6. Bot をチャンネルに招待

Slack の Bot は参加済みのチャンネルしか読み書きできません。対象チャンネルで実行:

```text
/invite @gwt
```

（`gwt` はアプリ名に置換）。招待前は Board に
`conversations.history error: not_in_channel` が表示されます。招待後は gwt の Board
からの投稿が Slack チャンネルに反映され、チャンネルのメッセージが Board に表示されます。

> OAuth コールバックポートが必要なのはサインイン時だけです。トークン保存後の Board
> 読み書きはトークンのみで動作するため、以降はポートが変わったり塞がっても既存
> セッションには影響しません（再サインイン時のみ登録済みリダイレクト URL が必要）。

### Microsoft Teams を Board バックエンドにする（実験的）

> Teams 対応は実装済みだが実テナントでの end-to-end 検証は未実施。以下は Microsoft
> identity / Graph の要件に基づく手順です。

#### 1. Entra (Azure AD) アプリを登録

1. <https://entra.microsoft.com> → **アプリの登録 → 新規登録**。
2. 名前 `gwt`（シングルテナントで可）。
3. **リダイレクト URI**: プラットフォームで **「モバイルおよびデスクトップ
   アプリケーション（パブリック クライアント）」** を選び、**正確に**入力:

   ```text
   http://127.0.0.1:8765/oauth/callback
   ```

   - `127.0.0.1`（gwt が送るホスト）を使い、gwt の OAuth callback port（既定
     `8765`）に合わせる（loopback ではポートが照合で無視されるため
     `http://127.0.0.1/oauth/callback` でも可）。
   - ポータルが http-loopback を拒否する場合は、アプリの **マニフェスト** で
     `replyUrlsWithType` に `"type": "InstalledClient"` として追加。
   - ⚠️ **「Web」で登録しないこと** — public client のトークン交換はシークレットを
     送らないため、Web 登録だと `AADSTS invalid_client` で失敗します。
4. **認証 → 詳細設定 → パブリック クライアント フローを許可する → はい**。

#### 2. Microsoft Graph 委任アクセス許可を付与

**API のアクセス許可 → アクセス許可の追加 → Microsoft Graph → 委任**:
`ChannelMessage.Send` / `ChannelMessage.Read.All` / `Channel.ReadBasic.All` /
`offline_access`。テナントが要求する場合は管理者の同意を付与。

#### 3. team_id / channel_id を取得

Teams でチャンネル → **チャンネルへのリンクを取得**。URL の `groupId=<GUID>` が
**team_id**、`/channel/` 直後の URL デコードした `19:...@thread.tacv2` が
**channel_id**。gwt の **Default channel** は `<team_id>/<channel_id>`。
（または Graph Explorer: `GET /me/joinedTeams` → `GET /teams/{id}/channels`。）

#### 4. gwt で設定 → サインイン

**Settings → Board provider → Teams** に **Application (client) ID** /
**Tenant ID** / **Default channel**（`team_id/channel_id`）を入力 → **Save** →
**Sign in**。投稿はサインインユーザー名義（Graph 委任。app-only 投稿は非対応）。
対象 team/channel に**参加している**必要があります（未参加だと Graph が `403` を返し、
gwt が対処メッセージを表示）。

## キャンバス操作

- 画面上の zoom ボタンでキャンバスを拡大・縮小
- 背景ドラッグでキャンバスを移動
- `Tile` で表示中のウィンドウをグリッド整列
- `Stack` でタイトルバーを残したまま重ねて表示
- `Align` でウィンドウサイズを変えずにグリッド整列
- `Cmd/Ctrl+Shift+Right` と `Cmd/Ctrl+Shift+Left` でフォーカス切替
  - フォーカスされたウィンドウは中央へ寄ります

## Operator デザイン言語 (SPEC-2356)

Operator Design System 採用後、gwt は editorial-industrial 系タイポグラフィ
(本文 `Mona Sans` / ディスプレイ `Hubot Sans Condensed` / 等幅 `JetBrains Mono`)
を中核とした単一の mission-control サーフェスとして再設計されました。
既定の type scale は開発時の可読性を優先し、terminal text、ID、path、
counter、密度の高い作業サーフェスを長時間でも読み取りやすくします。
一方で display typography は見出しと chrome label に限定して Operator らしさを
保ちます。Project Bar / Sidebar Layers / Status Strip / Command Palette /
Hotkey Overlay / Drawer モーダル / フローティングウィンドウ の全クロームが
共通トークンを参照し、 2 つの旗艦テーマで提供されます:

- **Dark Operator** (Mission Control / carbon + neon) — 既定、 長時間作業向け
- **Light Operator** (Drafting Table / bone + ink) — 明るい環境向け

OS の `prefers-color-scheme` に追従しつつ、 Project Bar の **Theme** control で
`auto` / `dark` / `light` を選べます。 選択はブラウザストレージに
永続化され、 再起動後も維持されます。 xterm の端末本文は可読性のため
Dark Operator palette に固定し、開発向けに大きめの font metrics を使います。
端末 window の chrome は overall theme に追従します。
Workspace Overview と Release Notes のような Quiet Work UI サーフェスでは、
status-board レイアウト、個別 fixed overlay、本文への display font 適用を避けます。
Workspace Overview は List + Detail の作業サーフェス、Release Notes は共通の
app-global window chrome を使い、このルールは SPEC-2356 と frontend UI contract
test で検証されます。
`prefers-reduced-motion: reduce` を有効にすると Living Telemetry
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
| `⌘\` | Sidebar Layers を折り畳み / 復帰 |
| `Esc` | 開いている Palette / Overlay / Drawer / Dropdown を閉じる |

### アクセシビリティ

すべてのモーダルダイアログ (Command Palette / Hotkey Overlay / Branch
Cleanup / Worktree Migration / Launch Wizard / Add Window) は WAI-ARIA
dialog convention に従います。`role="dialog"` + accessible name、
`aria-modal`、open 時にフォーカスがダイアログ内へ移動し close 時に
トリガーへ戻る、Tab がダイアログ内で循環 (キーボードトラップなしの
escape)、Esc で dismiss。非同期ロード段階は `aria-busy="true"` で
スクリーンリーダーに進捗を伝えます。エラー領域は `role="alert"` で
即座に読み上げられます。WCAG 2.1 AA コントラストは両テーマの全
text/surface 組合せでテストレイヤーに pin されています。

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
node scripts/test_release_assets.cjs
```

### Frontend Bundle Contract

```bash
bash scripts/check-frontend-bundle.sh
```

### Release Flow Checks

```bash
bash scripts/check-release-flow.sh
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
└── scripts/            # リリース、検証、メンテナンス用スクリプト
```

## SPEC

詳細仕様は `gwt-spec` ラベル付き GitHub Issue にあります。ローカルキャッシュ経由で
`gwtd issue spec <n>` を使って確認できます。

## ライセンス

MIT
