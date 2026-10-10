# gwt

gwt は agent-driven development のためのデスクトップ control plane です。
コーディングエージェント、プロジェクト文脈、共有 coordination、GitHub
Issue ベースの SPEC、セマンティック検索、managed workflow automation を、
ネイティブ GUI とブラウザから開ける 1 つの workspace に集約します。

Git worktree は gwt の背後にある隔離基盤です。gwt は安全な task ごとの
Agent workspace を materialize するために worktree を使いますが、利用者の
主導線は branch 管理ではなく、作業、Issue、SPEC、検索、Board 文脈から始まります。

## gwt の特徴

- **Agent workspace** — `Claude Code` / `Codex` / `Grok Build` /
  `Antigravity CLI` / `OpenCode` / `Copilot` /
  custom agent を共有 canvas から起動・再開・状態確認できます。
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

- GUI 向けの主配布物:
  - Apple Silicon: `gwt-macos-arm64.dmg`
  - Intel Mac: `gwt-macos-x86_64.dmg`
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

### アップグレード下限

一回限りの移行処理を廃止する際の upgrade floor は **v9.72.1** です。
2026-10-02 の変更日から60日前、2026-08-03 UTC 時点の最新リリースを基準にしています。
それより古い環境では、先に [v9.106.0](https://github.com/akiojin/gwt/releases/tag/v9.106.0)
をインストールし、各プロジェクトを開いて既存の移行を実行してから新しい版に更新してください。
更新前に gwt の設定とプロジェクト状態をバックアップしてください。

旧 Claude Code backend 行は例外で、公開版の起動経路から自動移行が呼ばれていませんでした。
**旧 backend 設定は自動移行されません。Settings で provider を再登録してください**
Settings → Agent Backends で旧設定の endpoint・API key・model を再登録し、
組み込みの Claude Code と登録した backend を選択してください。
旧設定は引き続き読み取り可能で、起動時に書き換えたり削除したりしません。

floor 以降に追加された Session schema 5、PM scratch、work-item projection rebuild v2、
ProjectKey の移行は維持します。
usage の `window_minutes` 契約と、未完了の SPEC #2359 に属する Workspace projection
backfill も維持します。旧 HOME / Workspace の `workspace/current.json`、
`work_items.json`、`journal.jsonl` からの取り込みは廃止しました。対応する現行ファイルが
ない場合、Workspace 状態の読み込み・保存を拒否し、旧ファイルのパスと更新手順を表示します。
旧ファイルは変更しません。v9.106.0 で各プロジェクトを移行してから更新してください。
空の現行ファイルを作る操作は移行になりません。現行 receipt・event の回復処理は維持します。
coordination のイベント取り込みと discussion の取り込みも、現用の回復処理と session 別
Stop 契約が利用するため保持します。旧 agent identity reset は廃止し、起動時には保存済みの
目的・進捗を保持します。`agent_identity.migration.json` は既存の内容を変更せず、
未作成なら新たに作成しません。

組み込みフロントエンドの cleanup リクエストには operation ID を必須とします。
更新前から開いているタブは再読み込みしてください。Launch Wizard は常に権限確認を省略し、
Fast mode を無効にして起動します。旧設定は保存内容を書き換えずに読み替えます。
Issue Monitor のプロファイルと直接の Session Resume は従来の設定を維持します。

## 前提

- `PATH` 上で `git` が使えること
- GitHub 連携機能を使う場合は `gh auth login` 済みであること
- gwt から起動する agent CLI が `PATH` 上にインストールされていること。
  Antigravity CLI は Google の native `agy` command として提供されます。

  ```bash
  curl -fsSL https://antigravity.google/cli/install.sh | bash
  ```

  Gemini CLI は組み込みエージェントから削除されました。旧 Gemini 設定と保存済み
  セッションは対象を名指しする警告を出して無視し、元ファイルは変更しません。
  カスタムエージェントによる独自コマンドの利用は引き続き可能です。

  Grok Build は xAI 公式の `grok` command で提供されます。
  `npm install -g @xai-official/grok` でインストールし、初回起動時に認証するか、
  API-key workflow では `XAI_API_KEY` を設定してください。
- エージェント利用時は必要な API キーを設定すること
  - `ANTHROPIC_API_KEY` または `ANTHROPIC_AUTH_TOKEN`
  - `OPENAI_API_KEY`
  - `GOOGLE_API_KEY` または `GEMINI_API_KEY`
  - `XAI_API_KEY`
- shared project index runtime の bootstrap / repair が必要な場合は
  Python 3.10+ が使えること

Linux デスクトップ版のビルドには WebKitGTK 系の依存が必要です。CI と同じ依存は
[docs/docker-usage.md](docs/docker-usage.md) を参照してください。

### 対応する組み込みエージェント

gwt は次の組み込みエージェントに対応しています。Launch Agent には、gwt が検出した
インストール済みの組み込みエージェントだけが表示されます。その他の CLI コマンドは
カスタムエージェントとして引き続き利用できます。

| エージェント | CLI コマンド |
| --- | --- |
| Claude Code | `claude` |
| Codex | `codex` |
| Grok Build | `grok` |
| Antigravity CLI | `agy` |
| OpenCode | `opencode` |
| OpenClaw | `openclaw` |
| Hermes Agent | `hermes` |
| GitHub Copilot | `gh copilot` |

## 使い方

`gwt` を起動するとタスクトレイ (macOS は menubar、Windows は notification
area、Linux は StatusNotifierItem 対応 DE のシステムトレイ) にアイコンが
常駐します。トレイメニューから操作します:

- **Open in browser**: 既定ブラウザで埋込サーバー (`http://127.0.0.1:<port>/`)
  を開きます。同じ URL は他のブラウザでも開けます。
- **Copy URL**: 起動中の tray プロセスの URL を OS clipboard にコピーします。
- **Projects**: 開いているプロジェクト、Recent の順に表示し、選んだプロジェクトの
  URL を開きます。実行中・エラー件数を表示し、開いているプロジェクトに
  エージェントのエラーがある間はトレイアイコンにエラーバッジが付きます。
- **About GWT**: 起動中の tray プロセスのブラウザ版 About / Version 画面を
  開きます。
- **Quit**: tray アイコン + 埋込サーバー + PTY 子プロセスを順に停止します。

プロジェクトのブラウザタブにはエージェントの RUN / BLOCK 件数と状態別 favicon を
表示します。BLOCK は待機・停止・エラーを含み、シェルは集計しません。
未読マーカーは、そのタブが可視かつフォーカスされたときに消えます。
Hub のタイトルと favicon は固定です。

ルート URL `http://127.0.0.1:<port>/` は **Hub** です。Open Folder、Clone from
GitHub、Recent projects、開いているプロジェクトの一覧を表示します。各プロジェクトは
固有の URL `http://127.0.0.1:<port>/p/<project-hash>` を持ち、プロジェクトへの
リンクは新しいブラウザタブで開くため、1 つのブラウザタブには 1 つのプロジェクトが
表示されます。異なるプロジェクトのウィンドウと起動ダイアログは独立し、同じ
プロジェクト URL を開いた複数のブラウザタブではワークスペースが同期します。
プロジェクト URL をブックマークやタブ復元で開くと、Recent のプロジェクトは自動で
開き直され、未知のプロジェクト URL は Hub へのリンク付きの「Project not found」を
表示します。プロジェクトのヘッダーにある **Hub** リンクは Hub を新しいタブで開きます。

Autostart は **Settings > System > Launch GWT at login** で切り替えます。
有効にすると `auto-launch` crate 経由で macOS LaunchAgent / Windows HKCU
Run registry / Linux XDG autostart を user scope に登録し、次回 OS ログイン時に
`gwt` が tray-resident process として起動します。ブラウザは自動では開きません。

```bash
gwt                                 # トレイ常駐 + 埋込サーバー起動 (loopback)
gwt --bind 0.0.0.0 --port 60745     # 埋込サーバーを LAN / VPN 到達可能な IP/Port に bind
gwt open                            # 起動中の tray インスタンスの Hub URL を既定ブラウザで開く
gwt open ~/src/my-repo              # そのプロジェクトを (必要なら開いてから) /p/<hash> URL で開く
```

`--bind <ip>` の既定値は `127.0.0.1` です。`--port` を省略し、保存値がまだ
ない場合は利用可能なポートを bind し、実際のポートを保存して次回以降も
再利用します。保存済みポートが使用中の場合は別のポートを選び、保存値を
更新して警告を出力します。明示的な `--port <n>` はその起動だけに適用され、
ephemeral port を選ぶ `--port 0` を含め、保存済みの暗黙ポートを変更しません。
同一 LAN や VPN-extended LAN の別端末からブラウザ UI に接続したい場合は
`--bind 0.0.0.0` を指定してください。運用者が選んだ既知のポートを使う場合は
`--port` を併用できます。`--no-tray` はトレイを登録しない一時サーバーを起動します。
起動元の親プロセスが終了するか、最後のブラウザセッションが閉じて5秒経つと終了します
（この猶予中は再読み込み・再接続できます）。ブラウザが一度も接続していない場合は
親プロセスの寿命に従います。`--no-open` はブラウザの自動起動を明示的に抑止します。
フラグなしの起動も、既定でブラウザを自動では開きません。

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

`gwtd` operation は stdin JSON envelope で処理され、GUI は起動しません。

```bash
gwtd <<'JSON'
{"schema_version":1,"operation":"issue.spec.section","params":{"number":1784,"section":"plan"}}
JSON

gwtd <<'JSON'
{"schema_version":1,"operation":"pr.current","params":{}}
JSON

gwtd <<'JSON'
{"schema_version":1,"operation":"board.show","params":{}}
JSON

gwtd <<'JSON'
{"schema_version":1,"operation":"daemon.status","params":{}}
JSON
```

`workspace.update` の応答を失った場合は、表示された `operation_id` を同じ Session の
`workspace.receipt`（`params: {"operation_id":"<UUID>"}`）に渡して確認できます。
この読み取り専用照会は Host へ接続せず、更新も再送しません。`applied` は永続化の完了、
`unconfirmed` は旧 Host を含め証拠がまだ確認できない状態であり、更新の失敗を意味しません。

`board.show` は、選択された workspace / session から見える最新20件を時系列順で
返します。`params.limit` に非負整数（例: `15`、`0` は空）を指定して件数を変更できます。
`params.all: true` は全宛先を対象にして既定上限を解除しますが、明示した `limit` が
常に優先します。provider の保持窓は残り、`all` は全履歴の読み込みを意味しません。
未知のキーは受け付けるキー一覧を示して拒否します。既存の `board` フィールドは維持し、
`page.total_entries` はCLI制限前の可視snapshot件数、`page.returned_entries` は
返却件数、`page.truncated` はCLI制限による省略の有無を示します。
`blocked` の各 entry は `escalation` オブジェクト（`resolved` 真偽値、`resolved_at`、
`resolved_by_entry_id`）を持ちます。escalation index に無い blocked entry は
`indexed: false` / `resolved: null` を返します。`params.unresolved: true`（既定 `false`）
を指定すると、未解決の blocked entry だけを返します。絞り込みは `limit` より先に適用されます。

返却サイズはおおむね「件数 × シリアライズされた1件のサイズ + metadata」です。
1件平均2 KiBなら20件で約40 KiBです。固定バイト上限はなく、長文ほど増え、
`all: true` では数百 KiB以上になる場合があります。

managed hook と runtime 委譲は `gwtd` を使います。macOS と Linux では、
ユーザーが JSON operation `daemon.start` を実行することでプロジェクトごとの
runtime daemon（Unix ドメインソケット IPC）が起動します。daemon
が live な間、同じリポジトリに繋がっている全 `gwt` インスタンスへ
イベントが fan-out されます（例: 片方のウィンドウで Board に投稿
した内容が、別インスタンスにも遅延なく届く）。Ctrl-C / SIGTERM で
daemon を停止するまでバックグラウンドで動き続けます。診断用に
JSON operation `daemon.status` で現在の endpoint を確認できます。
JSON operation `daemon.start` を実行していない場合は multi-instance fan-out は
無効ですが、ローカルのファイルベース state とファイル watcher は
従来どおり動作します。

Windows でも daemon は同じ形で動きます。GUI の Issue Monitor がユーザー
セッションの子プロセスとして起動・監視し、JSON operation `daemon.start` で
手動起動もできます。通信は named pipe（`\\.\pipe\gwtd-<scope>-<hash>`、
ローカルクライアントのみ。auth token は `~/.gwt` 配下の endpoint file が
持つ）です。`daemon.status` / `daemon.subscribe` / Issue Monitor control /
複数インスタンス間の fan-out は macOS / Linux と同じように動作します。
手動起動した daemon は Ctrl-C、Ctrl-Break、コンソールを閉じることで停止し、
ログオフとシャットダウンでも同じ cleanup が走ります。GUI が終了させた
daemon は次回起動時の liveness 判定で回収されます。gwt は Windows Service を
インストールしません。daemon が行うのは scan と claim までで、エージェント
pane の生成は GUI 側が担うため、Service 化してもヘッドレス自律実行には
ならず、ユーザー単位の `~/.gwt` state とも噛み合わないためです。ヘッドレス
自律実行は daemon の目標には含めていません。

## Agent Workflow

1. プロジェクトを開く、GitHub から clone する、または前回のプロジェクトを復元する
2. `Board`、`Issue`、Knowledge search surface で現在の作業、
   関連 owner、過去の判断を把握する
3. **Curate** レーンでは、Command Rail または Command Palette の `Intake` を
   選ぶ。branchless で使い捨ての session で、議論・計画し GitHub Issue を
   登録する。設計が必要な作業は同じ Issue に `gwt-spec` design-required label と
   SPEC artifact を付ける。Intake は branch を作らない
4. **Execute** レーンでは、登録済みの作業を実行する。`Open Workspace` が既存
   branch 上で `Agent` を起動する、背後の `Issue Monitor` が登録済み Issue を
   自動で拾う、または owner が既知なら Issue detail から `gwt-execute #N` で直接 Launch Agent する
5. Execute 起動が確定した時にのみ、gwt が背後の `work/YYYYMMDD-HHMM[-n]`
   branch / worktree を materialize する（Intake session は branchless で ephemeral）
6. Agent 実行中は shared Board に status、claim、next、blocked、
   handoff、decision を残して coordination する。Board 投稿を Slack / Teams
   にも流したい場合は、先に remote Board provider を設定します。手順は
   [Board プロバイダ](#board-プロバイダ-local--slack--teams) を参照してください
7. Git の確認・filter・cleanup・低レベルな branch/worktree detail が必要な場合だけ
   `Branches` を開く

主なウィンドウ:

- `Agent` — Intake / Open Workspace / Issue Monitor / Launch Agent から作成される実 coding-agent process window
- `Board` — reasoning と coordination のための user / agent shared timeline
- `Issue` — semantic search、detail pane、design-required tag、Launch Agent handoff
  を備えた cache-backed Work Item Knowledge Bridge。legacy `SPEC` window も同じ
  Work Item view を開きます。Issue Monitor の自動起動はキャンバスにウィンドウを
  開かず、Issue ウィンドウの右ペインに読み取り専用でミラー表示されます。入力
  したいときは `Windowize` で通常のウィンドウにできます。各行には対応する Work
  の lifecycle・注意理由・PR 状態が表示され、`Continue work` / `Resume` /
  `Clean Up` をその場で実行できます
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
Work Item Knowledge Bridge は frontend から GitHub API response を直接描画せず、
ローカル Issue cache と semantic index を使います。

Windows の Host 起動では、Launch Agent で Command Prompt、Windows PowerShell、
PowerShell 7 を選択できます。Docker 起動では引き続きコンテナ内のシェルを使います。

ターミナルウィンドウでは、テキストをドラッグ選択してマウスボタンを離すとコピー
できます。Windows では `Ctrl+C` で現在の選択をコピーして選択を解除します。
選択がない場合、`Ctrl+C` は実行中のターミナルプロセス向けの割り込みのままです。
Linux では `Ctrl+Shift+C` でも現在の選択をコピーできます。

## Issue サーフェスと Issue Monitor

Add Window から `Issue` を開くと、キャッシュ済み GitHub Issue を Backlog / Queued /
Active / Done の 4 列で表示します。各行の実行状態と操作はそのまま利用できます。
Backlog から Queued へドラッグすると実行対象に追加され、逆方向でキューから外れます。
Queued 内では順序を変更でき、複数選択の移動は 1 回のキュー操作で送信されます。
Active と Done は実行ライフサイクルに従うため、ドラッグでは変更できません。
Queued には `auto-refill` などの追加元も表示されます。

検索と Kanban / Split の切り替えは同じ行に並びます。Monitor の状態表示と操作は分離し、
Settings、Autonomous、Auto-refill と上限、Start monitor / Stop をラベル付きで表示します。
Auto-refill は**既定で OFF**です。有効にすると、条件を満たす open Issue を設定した
キュー上限まで自動補充します。空のキューから新しい作業は起動せず、実行中の作業は継続します。
監視エラーは通知センターで確認できます。詳細ペインには受け入れ基準の進捗と状態別の
操作を表示します。カードを選び、**Issue / Output** で本文・受け入れ基準とエージェントの
読み取り専用出力を切り替えます。**Windowize** でエージェントを Canvas へ移せます。
**Hide preview / Show preview** でボードを全幅に広げたり、詳細ペインを再表示したりできます。
列は縮めず横スクロールします。従来の `issue_monitor` preset も同じ Issue サーフェスを開きます。

**Max active** は新規設定で **Auto** を使用します。推奨値には CPU、空きメモリとディスク、
GUI の CPU 使用量、他プロジェクトの稼働中エージェントを反映します。稼働中のエージェントが
ない登録済みプロジェクトは配分を消費しません。**Machine budget** には制約になった資源を
表示し、Monitor の実装・レビュー上限と PM を含む総数を区別します。必要な実測値が
得られない間、Auto は新規起動を待機させます。実行中のエージェントは継続します。
大きな `target` ディレクトリの初回実測には数分かかる場合があります。
更新中に前回の実測値が期限切れになった場合も、新規起動は待機します。
正の数値を入力すると **Manual** の上書きを保存し、**Use Auto** で推奨値への追従に戻せます。
既存の保存済み上限は Manual として維持します。推奨値を超える入力も許可しますが、
検証が完走しない可能性と、時間に依存するテストの失敗が無関係な PR を妨げる可能性を警告します。
自動化では `issue.monitor.config.set` に `{"max_active_mode":"auto"}` を渡すと Auto、
`{"max_active":4}` を渡すと上限 4 の Manual になります。`issue.monitor.status` は実効上限、
`max_active_agents_override`、共有実測値の `agent_capacity` を返します。

**Allowed labels** で、この端末の Monitor が拾う Issue をラベルで指定できます。
ラベルを1件ずつ追加・削除し、保存済みリストのいずれかに一致する Issue が対象になります。
大文字・小文字と前後の空白は区別しません。空のリストは全ラベルを許可し、従来の対象条件を
維持します。変更は次の scan で反映され、実行中のエージェントは中止しません。
設定欄には保存済みラベルと除外件数・Issue 番号を表示します。自動化からは
`issue.monitor.config.set` に `{"allowed_labels":["agent:mac"]}` を渡せます。
`issue.monitor.status` は `allowed_labels`、`label_excluded_count`、
`label_excluded_issues` を返します。

open な GitHub Issue は、明示的な追加、有効な Auto-refill、または `urgent` ラベルによる投入
まで Backlog に留まります。キューへの所属は Monitor の実行候補になる条件であり、
準備状態・claim・同時実行数のチェックは引き続き適用されます。行の `Launch now` は
起動フローを開き、起動時に `work/issue-N` のブランチ/worktree を作成して
`gwt-execute #N` でエージェントを開始します。起動失敗は Issue 行に残ります。

`urgent` は誰でも付与できます。実行候補の urgent Issue は Auto-refill が無効でも
自動投入されますが、明示的なキュー削除は優先されます。既定では付与順に最大2件が
先頭群へ入り、保存済みの通常順序と `max_active` は変わりません。
`issue.monitor.queue.urgent_limit` の `limit` で上限を変更できます（`0` は先頭群への昇格だけを無効化）。
超過分は通常順序に従います。`issue.monitor.queue.demote` の `number` で指定した Issue は
永続的に通常優先度へ戻り、scan・再起動後も urgent ラベルより降格が優先されます。
カードと詳細では urgent・上限超過・降格を区別し、`issue.monitor.queue.list` と
`issue.monitor.status` からも理由と観測済みの GitHub ラベル付与者・時刻を確認できます。
未取得の履歴は不明として表示します。

Agent や自動化からは `issue.monitor.status` で確認し、`issue.monitor.queue.push`、
`issue.monitor.queue.remove`、`issue.monitor.queue.move` で所属と順序を変更できます。
`issue.monitor.queue.auto_refill` は自動補充の有効化と上限を設定します。
`issue.monitor.launch_now` は対象を端末キューの先頭へ明示的に追加し、scan を要求します。
既存の `issue.monitor.priority.move` と `issue.monitor.priority.set` も利用できます。
`issue.monitor.config.set` は処理停止、Autonomous
モード無効化、正の `max_active` 上限設定に対応します。安全のため `enabled=true` と
`autonomous_mode=true` は拒否され、有効化には GUI での明示操作が必要です。
idle になったエージェント窓はスロットを自動的に解放します。各 scan は起動中の窓を
`review_verdict_published` / `execution_settled` / `binding_dead` /
`stuck_unknown` に分類し（`issue.monitor.status` の行と `idle_windows` で確認可能）、
前 3 種は解放して pane を閉じます。解放された Issue は通常 queue に戻りませんが、
エージェントが実行を settle する前に窓が失われた場合（アプリ再起動が pane ごと落とした
場合など）は requeue され、次の scan が既存ブランチのまま再 launch します。実行レコードが Active の
まま idle な `stuck_unknown` だけは人の判断に残り、stuck タイムアウトの 2 倍を超えると
判断を求める通知を出します。`issue.monitor.release_idle` は同じ解放を Issue 単位
または全 idle 行に対して手動実行し、`dry_run: true` は対象の報告だけを行います。
`issue.monitor.profiles` は起動候補プールを返し、`issue.monitor.profiles.set` は
プールを置き換えます。候補が 2 件以上あると、Monitor は各 Issue を最初の適格な候補
で起動する（rate limit の hold と `prefer_for` routing が適格性を決め、provider は
使用率の読み値による予測ではなく、起動を拒否した時点でプールから外れる。
詳細な規則は SPEC [#3914](https://github.com/akiojin/gwt/issues/3914) に定義）
ため、1 つの provider が rate limit に入ってもキューは止まりません。
`issue.monitor.status` は provider ごとの最新の使用率の読み値を `provider_usage` に
返し、読み値が無い場合はその理由を返します。レートリミットの初回拒否で provider を
hold し、全候補が hold 中なら最も早い既知の reset 時刻に自動再開します。
reset がすべて不明なら定期再試行せず、`needs_human_fleet` の
`launch_candidates_exhausted` として通知します。GUI の Issue Monitor 設定フォーム
（`⚙ Settings`）は同じプールを Agent Settings の組として並べ、`＋` で組を追加、
`−` で削除、矢印で並べ替えができ、保存した並び順がそのまま起動候補の順序になります。
各 operation
は省略可能な `project_root` を受け取り、省略時は現在の worktree を対象にします。
Priority の変更と daemon 不在時の設定変更は、実行中 instance の next scan/rebase で
反映されます。

`issue.monitor.tiers.set` に `{"auto":true}` を渡すと、エージェント・モデル・
推論レベルの自動選択を有効にできます。候補プールを設定しなくても、Codex Luna /
Claude Haiku、Codex Sol / Claude Sonnet、Codex Astra / Claude Opus の3段を使えます。
段は0始まりで、エージェントの成果に由来する失敗回数・Issueに保持した下限・SPEC Issueなら1の最大値を使い、
最上段で頭打ちになります。再試行の許可条件と終端処理は従来どおりです。
各段のprovider選択は既存の候補選択規則を使い、候補がなければ次の段へ進みます。
通常の自動起動は、現在のモデルと推論レベルを反映するため新しいセッションを使います。
回答済みhandoffは元のセッションへ配送し、段の履歴は変更しません。
インフラ障害や終了証拠のない終了では段を上げません。ただし、従来の再試行予算と
backoffに使う総試行回数には数えます。

`issue.monitor.tiers.set` の `tiers` にプロファイル配列の配列を渡すと段を変更でき、
省略すると既定に戻ります。`{"auto":false}` で保存済みの手動プールに戻ります。
`issue.monitor.tier.set` の `number` と `tier` でIssueの段の下限を引き上げ、
次回起動にも保持できます。`issue.monitor.tiers` は設定・Issue履歴・最下段での
着地率を返します（着地の観測がなければ `null`）。判定不能な終了の累計も確認できます。
`issue.monitor.status` の各行には `launch_tier` と `landing_tier` に加え、総試行回数の
`attempts`、段に数えなかった `non_agent_attempts`、差分の `tier_input` が表示されます。
着地は、担当Issueとセッションが一致するWorkの `done` 更新成功を指します。
Issueのcloseやcancelだけでは数えません。エージェントによる完了宣言を測るため、
後からPRが失敗すると着地率は楽観的になります。この指標にPRのマージは不要です。

ホストの空き容量も同じ snapshot に含まれます。`issue.monitor.status` の
`disk_space` は worktree と verification coordinator が置かれた volume を列挙し、
空きが 20 GiB または 5% を下回ると `warning` を載せるため、`verify.run` が
`No space left on device` で落ちる前にディスク枯渇が見えます。空き容量の回収は
`worktree.gc_build_artifacts` operation が行います。HEAD が `origin/<base>`
（`base` の既定は `develop`）にマージ済みで、稼働中プロセスも live な gwt launch も
無い worktree の `target/` ビルドキャッシュを削除します。引数無しの呼び出しは dry run
で、候補とそのサイズ、および除外した worktree とその理由（`active process …` /
`tracked launch …` / `not merged …`）を報告します。削除するには `dry_run: false`
を、未マージの idle worktree も対象にするには `include_unmerged: true` を渡します。
共有の base ブランチ workspace（`develop` / `main`）は、リビルド代償を次に触る人が
負うことになるため既定で除外され、`include_protected_workspaces: true` を明示した
場合のみ対象になります。稼働中の worktree、main worktree、呼び出し元の worktree、
実行中の `gwtd` を置く worktree には、どのフラグを渡しても決して触れません。

低容量時の自動 GC は、マージ済みの idle キャッシュを先に回収します。それでも
`[build_artifact_gc]` の設定閾値を下回る場合は、空き容量を1件ごとに確認しながら
未マージの idle キャッシュを回収します。両段階とも生存プロセスと launch を保護します。
回収量がゼロなら、`issue.monitor.status` の `build_artifact_gc` に
`outcome: no_reclaim`、`warning`、`kept_by_reason` を出し、列挙の成功だけを
回収成功として扱いません。すべてのキャッシュが使用中の場合、GC だけでディスク枯渇を
防げる保証はありません。

Workspace パネルの `Clean Up Ready` 件数も、worktree 単位で同じ考え方を使います。
マージ済みまたは差分の無い Workspace は、未コミットの差分が gwt 自身の書き込み
（`.gwt/` namespace、materialize された `gwt-*` skill / command、手書きの内容を含まない
`.codex/hooks.json` / `.claude/settings.local.json`）だけであれば cleanup-ready のまま
数えられます。それ以外の未コミット変更があれば、その Workspace は件数から外れます。

### プロバイダの無料リセット

`provider.reset.proposals` は provider hold を読み、残り待機時間が
`min_reset_wait_secs`（既定: 86400 秒）以上なら Codex の無料リセット枠の確認を
提案します。Claude の上限時は別プロバイダへの切替を提案します。
**Claude の追加利用は課金を伴うため gwt からは実行しない**方針です。

`provider.reset` に `provider: "codex"` と `pane.list` の正確な `window_id` を
指定すると、無料枠を確認した後に OS の確認ダイアログを表示します。
**Redeem free reset** を選ぶと、そのアカウントの無料リセットを1回消費します。
既定は Cancel で、autonomous モードでも確認を省略しません。
JSON の承認フラグや過去の承認は利用できません。canvas 上にある、直接インストールした
Host Codex と既定プロバイダの window に対応し、Docker・package runner・独自 backend は拒否します。
確認画面には対象窓の起動時に記録した認証ルートと由来（host・profile・caller environment）を
表示し、ヘルパーも同じルートを使用します。証跡のない古い窓は再起動してください。
認証環境を解決できない場合は実行を拒否します。
Windows では対象窓を起動する前に `CODEX_HOME` を明示してください。

成功後はアカウントの利用再開を再確認し、provider hold を自動解除します。
失敗や結果不明の場合は hold を保持します。承認・実行・結果は
`~/.gwt/provider-resets/<request_id>.jsonl` に保存し、operation はそのパスと失敗理由を
返します。リセットと hold 解除の成功後に最終監査の書き込みだけが失敗した場合は、
成功結果を維持し、`audit_warning` を別に返します。クレジット購入や有料追加利用への切替は行いません。
引数の詳細は `gwtd --help provider` を参照してください。

### Autonomous モード（opt-in）

Autonomous モードはループ全体を無人で実行します: 適格 Issue → 自動起動 → 実装 →
独立レビュー → 強い自動ゲート → 自動マージ。**既定では無効**で、**二段階の
opt-in** が必要です:

1. Issue サーフェスの `Autonomous` トグルを有効化（プロジェクト単位）。
2. 自律処理したい各 Issue に `auto-merge` ラベルを付与。

さらに、機械検証可能な受け入れ基準（本文の `## Acceptance Criteria`
チェックリスト）があり、ベースブランチの protection ルールが検証可能で、
上限つきの試行回数を使い切っていない Issue だけが適格になります。それ以外は
従来どおり human-gated のまま扱われます。

安全モデルを一行で: マージ判断を実装エージェント自身には決して委ねません —
独立レビューと強い自動ゲートの通過が必須で、失敗は可視の `NeedsHuman` 状態に
エスカレーションし、`Autonomous` トグルは monitor が arm した auto-merge を
能動的に解除する kill switch として機能します。ゲート設計と脅威モデルの全体は
SPEC [#3200](https://github.com/akiojin/gwt/issues/3200) を参照してください。

work ブランチが `develop` に merge されると、monitor は delivered な Issue を
自分で決着させます（`Closes #N` は default branch でしか発火しません）。
受け入れ基準がすべてチェック済みか、PR 本文 / Issue コメントに残りの基準を
別 Issue に委譲した記録（`残 AC は別 Issue に委譲`）があれば、PR 番号と merge
SHA を含むコメントを投稿して Issue を close します。未達の基準が残る場合は
`merge 済み・未達 AC あり` コメントを残して `NeedsHuman` にし、`gwt-spec` Issue は
全 Phase の tasks が完了したときだけ close します。auto-close は既定で
`Autonomous` トグルに連動し、`issue.monitor.config.set` の
`auto_close_merged_issues=true|false` で上書きできます（off のときは
`merge 済み・close 待ち` コメントの記録のみ）。人間が reopen した Issue を同じ
merge で再度 close することはありません。

無人運転中のライフサイクルイベント（マージ完了・再試行予約・ゲート通過・
NeedsHuman エスカレーション）はトーストとして表示され、永続的なスクロール可能
通知スタックに蓄積されるため、離席中のイベントも失われません。

Agent の状態通知は **停止**・**エラー**・**人間の対応待ち**を伝えます。
Idle を Work 完了とは扱いません。runtime のデスクトップ通知は、ページで連続した
Running を5分以上観測し、ページが非表示またはフォーカス外の場合に限ります。
別状態への遷移や再接続で計測をリセットします。Monitor の NeedsHuman は窓のない
Issue も既存の通知ストリームで即時に伝え、inbox snapshot から二重に通知しません。
Session Interrupted は再開候補の過去 snapshot としてしか公開されていないため対象外です。

ネイティブ権限の adapter は macOS の認可設定、Windows の通知設定、Linux の
認可照会不可を区別します。不明・未設定・拒否・照会失敗では配送を許可せず、権限を
自動要求しません。Linux の GetCapabilities はサーバー機能であり、ユーザーの許可では
ありません。この adapter 自体はタブなしのネイティブ配送を追加せず、その配送機構は
SPEC #3287 の別の実装範囲です。debug binary では判定とブラウザ挙動を検証でき、
署名済み macOS bundle の権限・配送と Windows/Linux のネイティブ操作は実機での別検証が必要です。

調整可能な上限（試行回数・stuck/idle タイムアウト・再試行バックオフ・レビュー
モデル）はプロジェクト単位で永続化されます。human-gated の基礎は SPEC
[#3165](https://github.com/akiojin/gwt/issues/3165) を参照してください。

## PM エージェント

各プロジェクトには常駐の **PM エージェント**ペインが 1 つ起動します。これが
ユーザーの唯一の対話窓口です。自然言語で要望を伝えると、PM が Issue への分解・
登録・design-required Issue の計画策定・意味的な実行順序の決定・Issue Monitor
への起動指示までを行います。進捗の報告と `NeedsHuman` エスカレーションの提示も
同じ会話の中で行われます。

PM 自身は実装エージェントを起動しません。対象 Issue をキュー先頭へ移動して
スキャンを要求するだけで、実際の起動は Issue Monitor の既存 claim/slot 経路が
担うため、多重起動の防止機構はそのまま維持されます。

- プロジェクトを開くと自動起動します。プロジェクト単位で opt-out できます。
- PM ペインを閉じると停止し、自動再起動はしません。クラッシュ時は自動復帰し、
  クラッシュループを防ぐバックオフが働きます。
- Issue Monitor の `enabled` / `autonomous_mode` を CLI から有効化できるのは
  PM だけです。他のエージェントセッションは GUI 操作が必要です。マージ判断は
  影響を受けません — 上記の強い自動ゲートが引き続きすべてのマージを決めます。
- PM は project store 単位ではなく**リポジトリ単位**で 1 つです。state が
  2 つの store に分かれたリポジトリでも PM は 1 本だけになります。JSON operation
  `pm.status` はリポジトリ内の全登録を一覧し、`pm.stop` は CLI から PM を
  停止します。登録済み PM は孤児化した PM を停止でき、自分自身も GUI 操作なしで
  停止できます。

設計は SPEC [#3431](https://github.com/akiojin/gwt/issues/3431) にあります。

## Knowledge、Search、Managed Skills

gwt は project knowledge を Agent workspace の近くに置きます。

- JSON operation `issue.spec.read` は GitHub Issue-backed SPEC をローカル cache から読みます。
- JSON operations `issue.view` と `issue.comments` は gwt CLI surface から
  cache-backed Issue access を提供します。
- `gwt-search` は共有 ChromaDB runtime を通じて SPEC、Issue、source files、
  docs を検索します。index が無い場合は必要に応じて build され、desktop app は
  管理対象 Python search runtime を修復できます。
- Work Item Knowledge Bridge は plain Issue と `gwt-spec` label 付き Issue の
  cache-backed list/detail、semantic ranking、exact match priority、match percentage
  を組み合わせます。

Bundled workflow skills は active worktree の `.claude/skills`、
`.claude/commands`、`.codex/skills` に materialize されます。公開 entrypoint は
以下です。

- `gwt-discussion` — investigation-first な議論と設計明確化
- `gwt-register-issue` — 作業 intake。plain Issue または design-required な
  `gwt-spec` Issue を作成する
- `gwt-plan-spec` — 承認済み SPEC の implementation planning
- `gwt-execute` — `#N` または承認済み task からの TDD-oriented implementation
- `gwt-build-spec` / `gwt-fix-issue` — 1 release cycle の transition alias
- `gwt-manage-pr` — PR create/check/fix lifecycle
- `gwt-arch-review` — architecture review と改善 routing
- `gwt-search` — unified semantic search
- `gwt-agent` — running agent pane の確認と操作

Managed hooks は user hook を保持しながら、Agent state、workflow guardrails、
Board reminders、discussion/plan/build Stop checks、coordination-event summaries
を追加します。

### Hook ファイルの所有権

- gwt は `.claude/settings.local.json` をマシンローカルファイルとして再生成し、
  Git 除外も gwt が管理します。
- gwt は `.codex/hooks.json` を作成またはマージしますが、`.gitignore` にも
  `info/exclude` にも追加しません。
- `.codex/hooks.json` を version 管理するかどうかはリポジトリ側の決定です。
  ファイルが既に存在する場合、gwt は gwt-managed hook エントリだけを差し替え、
  user hook と無関係な top-level 設定は保持します。
- gwt リポジトリ自身では `.codex/hooks.json` を Git 除外し、gwt がエージェント
  セッションを準備するときにローカルで生成します。Windows は PowerShell の
  EncodedCommand、macOS / Linux は POSIX shell のコマンドを使用します。
  この生成ファイルを追跡しないことで、OS による違いが作業ツリーの差分に残るのを防ぎます。
- version 管理する場合は移植可能な `gwtd` fallback を維持し、マシンローカルの
  絶対パスをコミットしないでください。再生成は
  `GWT_HOOK_BIN=gwtd cargo run -p gwt-skills --example regenerate_hook_settings -- worktree-local`
  で行います。
- launch 以外では、gwt は Codex の hook discovery 先を両方
  （worktree ローカルの `.codex/hooks.json` と repo root 側の workspace-home
  コピー）所有します。hook health の報告と self-heal は常に同じファイル集合を
  対象にします。

### Codex 推奨設定

gwt は GUI 起動のたびに、ホストの Codex 設定（`$CODEX_HOME/config.toml`、
既定は `~/.codex/config.toml`）に gwt 推奨の
`features.context_management.experimental_mode = true` が入っていることを
保証します。この設定は、コンテキストを単一の要約へ繰り返し圧縮する代わりに、
メモと検索可能な履歴として蓄積された詳細を保持します。gwt が書き込むのは
キーが未設定の場合だけで、ファイル内の他の table はすべて保持され、既に
キーを持つ config は書き換えられません。無効化したい場合は `config.toml` に
明示的に記述してください:

```toml
[features.context_management]
experimental_mode = false
```

gwt は明示された値（`true` / `false` を問わず）を尊重し、変更しません。
config.toml が parse 不能または書き込み不可でも起動は止まらず、path と原因が
error ledger（`errors.list`）に記録されます。

`errors.list` は既定で project スコープのエラーを返し、`project_root` で
プロジェクトを絞り込めます。起動時の共通設定などマシン全体のエラーは
`scope: "host"`、帰属不明の記録は `scope: "unknown"`、全スコープは
`scope: "all"` を明示して取得します。`project_root` を指定した場合は
host・unknown 行を含めず、所属プロジェクトを推測で補いません。

0.153.0 より前の Codex CLI は `[features]` 配下の table を読めません。
`[features.context_management]` が 1 つあるだけで config 全体が読めなくなり
（`invalid type: map, expected a boolean`）、`codex login` も起動しなくなります。
gwt が起動する codex と `PATH` 上の `codex` は別の version であり得るため、
gwt は起動時に `PATH` 上の codex（`codex --version`）を確認します。それが
0.153.0 より古い、または version を読み取れない場合、gwt はキーを書き込まず、
既存の `[features.context_management]` table を削除してその codex が動き続ける
ようにします。`PATH` 上の codex を 0.153.0 以降に更新すると、次回の gwt 起動時に
キーが再び書き込まれます。

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

最短手順は、**Slack** または **Teams** を選び、provider の **Default channel**
を保存してサインインし、そのチャンネルに bot またはサインインユーザーがアクセス
できる状態にすることです。Default channel が基本の Board 関連付けで、Workspace
別の指定がない投稿はそこへ送られます。

### Workspace を Slack/Teams チャンネルに関連付ける

Remote provider は Board 投稿の送信先チャンネルを次の順で決定します:

1. 投稿の最初の Workspace audience に対応する `channel_map`
2. provider の `default_channel`

Workspace audience がない投稿は `default_channel` を使い、General thread に配置
されます。gwt は Workspace/channel の組ごとに remote root message を 1 つ作成し、
root id を `.gwt/work/board-remote-roots.jsonl` に保存します。このファイルと
対応する `.gitattributes` の `merge=union` ルールを git に含めることで、他の端末や
Agent も同じ thread を再利用できます。

Settings UI から編集できるのは default channel です。Workspace 別に送信先を分ける
場合は、`~/.gwt/config.toml` を編集します:

```toml
[board.slack]
channel_map = { "workspace-id" = "C0123456789" }

[board.teams]
channel_map = { "workspace-id" = "team_id/channel_id" }
```

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

#### 3. チャンネルリンクをコピー

Teams でチャンネル → **チャンネルへのリンクを取得**し、リンクをコピーします。gwt は
保存時に `groupId=<GUID>` と `/channel/` 直後の URL デコードした
`19:...@thread.tacv2` を解析します。Teams リンクを取得できない場合は Graph Explorer
（`GET /me/joinedTeams` → `GET /teams/{id}/channels`）で同じ値を取得し、
`config.toml` に `[board.teams].default_channel = "team_id/channel_id"` を設定します。

#### 4. gwt で設定 → サインイン

**Settings → Board provider → Teams** に **Application (client) ID** と
**Tenant ID** を入力し、**Teams channel link** にリンクを貼り付けて
**Save** → **Sign in**。gwt は内部的には既存互換の `team_id/channel_id` 形式で
保存します。投稿はサインインユーザー名義（Graph 委任。app-only 投稿は非対応）。
対象 team/channel に**参加している**必要があります（未参加だと Graph が `403` を返し、
gwt が対処メッセージを表示）。

## PM のプロジェクト設定

常駐 PM は gwt 所有の runtime ディレクトリで起動します。リポジトリの skill、
hook、`AGENTS.md`、`CLAUDE.md` は project の data として読めますが、PM の設定には
読み込まれません。実装 agent は従来どおり project 設定を使います。既存の PM 会話は
自動移行せず、次回の PM セッション起動時から分離されたディレクトリを使います。

project 固有の規約を明示的に渡すには、
`~/.gwt/projects/<project-hash>/project-state/pm.json` の他のフィールドを保持したまま、
`settings.project_policy_files` を設定してください。
例: `"project_policy_files": ["docs/pm-policy.md"]`。パスは PM の project checkout からの
相対パスで、既定は空リストです。選択した内容は managed asset の再生成時に既存の
`gwt-pm` skill へコピーされ、runtime に project への symlink は作りません。
指定を外した内容は次回の再生成で除去されます。

## キャンバス操作

- 画面上の zoom ボタンでキャンバスを拡大・縮小
- 背景ドラッグでキャンバスを移動
- `Tile` で表示中のウィンドウをグリッド整列
- `Stack` でタイトルバーを残したまま重ねて表示
- `Align` でウィンドウサイズを変えずにグリッド整列
- `Cmd/Ctrl+Shift+Right` と `Cmd/Ctrl+Shift+Left` で Canvas 上の Agent を
  状態順（running/starting → waiting/idle → その他）に切り替え
  - Agent 以外はスキップし、非表示の Agent タブは選択時に表示して、対象の
    Agent を中央へ寄せます

## Operator デザイン言語 (SPEC-2356)

Operator Design System 採用後、gwt は editorial-industrial 系タイポグラフィ
(本文 `Mona Sans` / ディスプレイ `Hubot Sans Condensed` / 等幅 `JetBrains Mono`)
を中核とした単一の mission-control サーフェスとして再設計されました。
既定の type scale は開発時の可読性を優先し、terminal text、ID、path、
counter、密度の高い作業サーフェスを長時間でも読み取りやすくします。
一方で display typography は見出しと chrome label に限定して Operator らしさを
保ちます。Project Bar / Command Rail / Status Strip / Command Palette /
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
| `Esc` | 開いている Palette / Overlay / Drawer / Dropdown を閉じる |

画面左端の Command Rail は常時表示です。上段に Intake (Curate レーン) と
Open Workspace (Execute レーン)、
中段にウィンドウ操作 (Tile / Stack / Align / ウィンドウ一覧 / Add)、
下段に Command Palette が並びます。Board と Logs はレール項目ではなく、
Add Window のプリセットメニュー・Command Palette・`⌘B` / `⌘L` のホットキー
から開けます。レール項目にホバーするとラベルと実際のショートカットが
表示されます。ウィンドウを閉じる操作
(タイトルバーの × / タブの ×) は常に確認ダイアログを経由するため、誤クリック
で実行中のエージェントが失われることはありません。

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
- Issue-backed Work Item は `gwt-execute #N` で実行します。design-required な Issue
  は implementation 前に `plan` と `tasks` が必要です。
- ローカルキャッシュ:
  `~/.gwt/cache/issues/<repo-hash>/`
- Managed agent integration files:
  `.claude/settings.local.json` と `.codex/hooks.json`
- SPEC 一覧を読む:

```bash
gwtd <<'JSON'
{"schema_version":1,"operation":"issue.spec.list","params":{}}
JSON
```

- SPEC 全体を読む:

```bash
gwtd <<'JSON'
{"schema_version":1,"operation":"issue.spec.read","params":{"number":1784}}
JSON
```

- セクション単位で読む:

```bash
gwtd <<'JSON'
{"schema_version":1,"operation":"issue.spec.section","params":{"number":1784,"section":"spec"}}
JSON
```

- レビューへ渡す前に SPEC artifact を lint する: FR / AS / T 番号、Traceability
  表との整合、supersede のインライン注記、section マーカー / roundtrip の健全性を
  検査し、結果を Intake Inspection Snapshot に記録し、Finding Disposition Ledger
  を seed して reviewer checklist を出力します。critical finding があると非ゼロ
  終了します。索引が参照する欠落 comment ID と、索引外の artifact comment
  （section・part 番号付き）も報告し、内容の書き換えや削除は行いません。
  section 書き込みはホストの同じ cache を使う writer を直列化し、索引更新直前に
  最新本文と比較します。別ホスト・外部 writer・通信結果不明は保証対象外です。

```bash
gwtd <<'JSON'
{"schema_version":1,"operation":"issue.spec.lint","params":{"number":1784}}
JSON
```

- 完了を宣言してよいかを判定する: 各 section が snapshot と一致する GitHub 実体
  readback を持ち、critical finding がすべて disposition 済みであることを確認します。

```bash
gwtd <<'JSON'
{"schema_version":1,"operation":"issue.spec.inspection.complete","params":{"number":1784}}
JSON
```

## ログ

Logs サーフェスの **Project** でそのプロジェクトのイベント、**Global** で
起動時およびプロジェクトに属さない診断を表示します。プロジェクトを切り替えても、
バックグラウンド処理のログは発生元プロジェクトに残ります。

- アプリログ:
  `~/.gwt/projects/<repo-hash>/logs/gwt.log.YYYY-MM-DD`
- 起動時および共通の診断ログ:
  `~/.gwt/logs/gwt.log.YYYY-MM-DD`
- セッション状態:
  `~/.gwt/session.json`
- プロジェクト単位のワークスペース状態:
  `~/.gwt/projects/<repo-hash>/workspace.json`

### セッション履歴

gwt は最近の Session 履歴を `~/.gwt/sessions/` に保持します。台帳の初回表示時と
以後24時間以上の間隔でバックグラウンド整理を行い、起動時の復元が無効な停止済み履歴を
30日間の非活動後に削除します。保存窓、実行中の処理、復旧、未完了の作業が必要とする
Session は保護します。古い書き込み一時ファイルの残骸も整理します。
保持方針と読めない記録の扱いは
[Issue #5025](https://github.com/akiojin/gwt/issues/5025) に記載しています。

### macOS のファイルシステム負荷と Spotlight

worktree の index watcher は、自身の macOS FSEvents stream から直下の
`target/` の子孫を除外します。監視開始後に `target/` が作られる場合にも適用されます。
親から `target` ディレクトリエントリ自体の変更通知が届く場合は、index path policy
で除外します。保証範囲はこの gwt stream であり、OS 全体の FSEvents や他アプリの
購読は停止しません。現在、この index watcher を production で起動する呼出元は
存在しないため、この除外だけで `fseventsd` 高負荷の原因を特定したとは扱いません。

既存 worktree は、**システム設定 → Spotlight → 検索のプライバシー**を開いて
その `target` ディレクトリを追加してください。新規 worktree は、最初のビルドで
`target` が作られた後に追加してください。macOS のバージョンごとの操作は
[Apple の Spotlight プライバシー設定ガイド](https://support.apple.com/en-gb/guide/mac-help/mchl1bb43b84/mac)
を参照してください。gwt が Spotlight 設定を自動変更することはありません。

gwt の診断と併せてホストの CPU 状況を確認できます。

```bash
gwtd <<'JSON'
{"schema_version":1,"operation":"diagnostics.cpu","params":{}}
JSON
```

`host_cpu` には最新の `fseventsd` プロセス標本と、標本数・採取間隔を表示します。
macOS では1秒間隔で3回採取し、同じプロセスが全標本で CPU 100% を超えた場合に
警告します。プロセスや標本を取得できなかった場合は低負荷と断定しません。
警告は観測結果であり、特定 worktree が原因である証明ではありません。
稼働中のファイル監視利用者と Spotlight のプライバシー設定を確認してください。

Spotlight のインデックス処理そのものは Issue Monitor の snapshot から読めます。
`issue.monitor.status` の `spotlight` は `mds_stores` プロセスとその CPU 率を
列挙し、100% を超えたプロセスがある場合に `warning` を載せます。worktree が
数百規模のホストでは、この daemon がエージェント本体を上回る CPU 消費者になり、
そうでなければ「ホストが重い」としか観測できません。Spotlight の無い
プラットフォームでは、プロセスも警告も無い状態でこのブロックを返します。

## アプリ更新の適用待ち

ダウンロード済みの更新は、エージェントの作業が終わるまで適用待ちになることがあります。
既存の drain は terminal convergence の15秒周期で安全条件を評価し、2回連続で静止を
確認した後、60秒の猶予を置いて適用します。
`autonomous_tuning.update_drain_notify_after_secs` は待機通知の繰り返し間隔です
（既定1800秒）。強制再起動の期限ではなく、時間経過だけでエージェントを終了しません。

`~/.gwt/logs/update-YYYY-MM-DD.log` に stage、待機・拒否理由、次の自動評価がある場合は
`next_evaluation_at` を記録します。同じ理由を毎 tick ログへ繰り返さず、プロジェクト別の
最新観測を評価ごとに更新します。観測時刻はアプリの生存保証ではありません。
アプリが停止した場合、記録された次評価時刻どおりに評価されるとは限りません。

JSON operation `release.status` の `pending_update_version` は保存済みローカルmanifestの版です。`pending_version`（remote release branchの未配信bump）とは
別の値です。payloadが消失した場合、`update_stage` は `payload_missing` となり再ダウンロードを案内します。
`update_wait` は対象版と一致する最新の待機観測、`last_apply_result` と
`last_apply_failure` は直近の適用結果を返します。`attempt` はresume marker内の回数であり、
通算試行回数ではありません。成功すると以前の失敗結果は置き換わります。
インストール失敗時には旧版のまま再起動することがあるため、再起動だけで成功と判断せず、
案内された回復操作と `observed_version` を確認してください。
適用処理中の重複要求は1回にまとめ、失敗後は明示的に再試行できます。

## 開発

### ビルド

```bash
cargo build -p gwt --bin gwt --bin gwtd
```

`browser-check` スキル（この checkout の隔離 GUI 検証）は `hook.doctor` の証跡を
読むために `jq` が `PATH` 上に必要です。gwt 自体の実行には不要です。

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
cargo install cargo-nextest --locked --version 0.9.146
cargo nextest run -p gwt-core -p gwt --all-features --test-threads=1
cargo test -p gwt-core -p gwt --all-features --doc
```

nextest は各テストを別プロセスで実行し、120秒でタイムアウトしたテストを失敗として後続を継続します。doctest は rustdoc で別途実行します。

### CI スループットの計測

Python 3.11 以上、認証済みの `gh`、対象 PR のマージ履歴を含むローカル Git
履歴を用意し、develop の直近 25 マージを収集して入力データを保存します。

```bash
python scripts/ci_throughput.py --repo akiojin/gwt --limit 25 --save target/ci-throughput.json
python scripts/ci_throughput.py --input target/ci-throughput.json
```

収集はこの checkout で実行します（別の場所からは `--root` を指定）。不足する
履歴は先に取得してください。shallow clone は `git fetch --unshallow origin develop`
で補えます。`--before 2026-10-08T00:30:00Z` はマージ時刻の上限を固定し、
その時刻も含めます。`--workflow lint.yml` は同じ収集・再計算経路で Lint を計測します。
保存データの再計算には GitHub 接続や Git 履歴は不要です。保存済みの基準値は次で再現できます。

```bash
python scripts/ci_throughput.py --input scripts/fixtures/ci-throughput-2026-10-08.json
```

JSON は PR 作成からマージまでの時間、各 PR の最終 head に対する最新成功
workflow attempt、マージあたりの base 同期回数、runner 待ち、job ごとの所要時間を
出力します。時間は分単位で、p50 は中央値、p90 は nearest-rank です。
workflow は `run_started_at` から `updated_at`、job は `started_at` から
`completed_at` を計測します。runner 待ちは依存 job のスケジューリング後の
`created_at` から `started_at` で、全 job と required job の分布を分けて示します。
欠測は取得不能として扱います。再実行で以前の実行時刻が再利用され、作成時刻が
開始時刻より後になった job は、実行時間を保持して runner 待ちを取得不能とします。

固定した 25 PR の基準値は、作成からマージまでの p50 **130.27 分**、Test attempt
の p50 **32.43 分**を再現します。base 同期は第二親が base の第一親履歴に属する
マージを数え、**115/25 = 4.6 回**です。従来の `Merge ... develop` 件名フィルターによる
**110/25 = 4.4 回**も併記し、独自件名の正当な同期 5 件との差を比較できます。

### フロントエンドの共有状態（SPEC-5016）

移行したフロントエンド domain は `web/ui-state-store.js` で immutable なデータを保持します。
受信ハンドラはモデルを更新し、各面は selector を購読して確定した snapshot を描画します。
取り外せる面では購読解除関数を保持し、DOM node と描画関数をモデルに含めません。
通知は非フォーカスの窓でも動き、focus や animation frame を更新条件にしません。
共通の `ui-content.js` は content の型から plaintext または backend で sanitize 済みの
Markdown 描画を選びます。移行対象と受け入れ条件は
[SPEC-5016](https://github.com/akiojin/gwt/issues/5016) を参照してください。

### 重量級検証の容量制御

ホスト全体の verification lease を取得するのは canonical `verify.run`
だけです。`verify.plan` で検証行列を登録し、`verify.run` で実行します。
各 Heavy コマンドの実行時に取得・解放し、Light コマンドは他の run と並行できます。
未実行の Light を先に進め、Heavy の相対順序を保ち、gwt の成果物復旧は最後に実行します。
残りの最初のコマンドの取得待機が時間切れになると、記録を置き換えず既存の記録を保持します。
途中の時間切れでは、先行コマンドの結果を未完了・非 PASS の `deferred` 記録に残します。

次の短命な non-Cargo ゲートは Light として Heavy lease を取らずに実行します。

| コマンド | 資源を限定できる根拠 |
| --- | --- |
| `git diff --check`（`--cached` を含む） | 差分の空白を検査する |
| `node scripts/check-coverage-threshold.mjs <summary> <threshold> ...` | 既存の coverage JSON を読む。テストは実行しない |
| custom checker 指定のない `actionlint`、`shellcheck`、`yamllint` | workflow・shell・YAML ファイルの静的解析 |
| `taplo check`、`taplo fmt --check` | TOML の検証・書式確認 |
| `typos` | 静的な綴り検査 |

これらのゲートには local・daemon 両方で 60 秒の実行タイムアウトを設けます。
時間切れは失敗（exit 124）として診断出力を残し、そのコマンドの process tree を停止します。
診断されたコマンドを修正してから行列全体を再実行してください。
既存の markdownlint・スコープ付き Cargo の分類は従来どおりです。
unknown コマンド、script wrapper、coverage を生成する `coverage-summary.mjs`、Cargo build・
広範囲の test、headed Playwright は Heavy のままです。
`actionlint -shellcheck` / `-pyflakes` の上書き指定も任意の wrapper を起動できるため Heavy です。
bounded command は時間切れ時に加え、通常終了時にも子孫プロセスを回収します。
Node reader も実効 `NODE_OPTIONS` が空でない場合は、任意 module を preload できるため Heavy です。
明示的な `NODE_OPTIONS=` は継承オプションを無効にし、Light 分類を維持します。

再試行には同じ要求行列全体と headed E2E の指定を渡します。`verify.run` は、owner・session・
execution authority・plan content hash・source fingerprint・要求コマンドが完全一致し、
先行コマンドがすべて signal なしで成功した、有効な admission-deferred 記録だけを自動再開します。
失敗・強制終了・クラッシュ・不一致などの記録では新しく実行し、登録 plan の不一致は
引き続き `verify.plan` の再登録が必要です。再開した証跡は不変の先行記録を id/hash で参照し、
元の開始時刻とコマンドごとの headed E2E・nextest・admission 証跡を保持します。
行列全体と必要な headed Chromium の dark/light 証跡が成功するまでは Overall PASS や Ready にはなりません。
独立した worktree とビルド directory の Heavy Cargo コマンドは、上限付きの
ホスト共通 pool を利用します。既定容量は論理 CPU 8 個あたり 1 slot と
メモリ 16 GiB あたり 1 slot の小さい方で、最低 1、最大 4 です。
`~/.gwt/config.toml` で上書きできます。

```toml
[verification]
slots = 4
```

同じ worktree または実効 Cargo target directory の実行は直列化します。
共有資源を特定できない wrapper と旧 binary は全体排他を使います。
各子プロセスの一時 directory を分離し、target と一時 directory の volume ごとに
GC の空き容量閾値を残してディスク容量を予約します。
`verification.disk_budget_bytes` で実測に基づく run 単位の予算を上書きできます。
既定の予約量は各 volume で 5,904,433,337 bytes（約 5.5 GiB）です。
target と一時領域の実測増分に 20% の余裕を加えて算出しています。
`verify.lease.status` は `capacity`、`running`、`available`、`slots` 内の保持者と ETA、
共通 FIFO queue を表示します。再試行前に確認してください。

```bash
gwtd <<'JSON'
{"schema_version":1,"operation":"verify.lease.status","params":{}}
JSON
```

各 `verify.run` は admission を待つ前に試行記録を作成します。
`verify.status` で自分の最新の試行を確認し、`params.attempt_id` を渡すと
特定の試行を確認できます。JSON 出力には試行 ID、状態、中断理由、
FIFO 予約が残っているかを含みます。

```bash
gwtd <<'JSON'
{"schema_version":1,"operation":"verify.status","params":{}}
JSON
```

不要になった試行は、返された ID と理由を指定して取り消します。

```bash
gwtd <<'JSON'
{"schema_version":1,"operation":"verify.cancel","params":{"attempt_id":"<attempt-id>","reason":"superseded matrix"}}
JSON
```

取消には同じ project・worktree・session・execution authority が必要です。
他者の試行は `not your verification attempt` として拒否します。
対象を `interrupted` と記録し、その試行の予約だけを直ちに解放して、
所有する command tree を停止します。runner が終了した場合も予約 TTL を待たずに解放します。
中断は PASS やテスト失敗とは区別し、同じ行列を再実行すると新しい試行を開始します。
最初のコマンド開始前に取り消した場合は、以前の検証記録を保持します。

初回の `cargo build -p gwt --bin gwtd`、通常の Cargo build、TDD テスト、
lint、coverage、直接の headed browser 確認、pre-push 確認は verification
lease なしでそのまま実行します。完了判定には引き続き canonical な検証証跡が
必要です。

`verify.lease.status` は `holder_project_relation`（`same_project` /
`other_project` / `unknown`）、`holder_reclaim_candidate`、
`holder_intervention` を判定として返します。他プロジェクトの holder、所有者を
特定できない holder、活動を判定できない holder は
`holder_intervention: forbidden` として保護されます。観測回数を増やしても
`unknown` の holder を停止する根拠にはなりません。同一プロジェクトの回収候補または
旧 control channel だけが `canonical_release_only` になり、canonical release が状態を確認します。
他プロジェクトからの release は拒否されます。拒否を `kill` / `pkill` で迂回しないでください。

ETA には `estimated_remaining_ms_uncertain: true` が併記されます。これはバッチの
推定時間または lease TTL で、現在の進捗を測るカウンターではありません。値が不変でも
固着の証拠にはなりません。`waiter_action: wait` は canonical admission を待つのが
正しい挙動だと示します。待機列に並んでいることは holder を停止する権限になりません。

`verify.run` はコマンド開始前に未完了の記録を保存します。本体が外部終了すると、
監視プロセスが中断理由、最後に実行中だったコマンド、完了したコマンドの結果を
記録します。`execution.status` は `running`・`interrupted`・`missing_record`
を区別します。中断記録では完了や PR 作成を許可せず、再実行が必要です。
診断用の `.gwt/tmp/verify-run.json` は原子的に書き込み、マシンローカルの
trusted record を引き続き正本とします。正確な終了シグナルを観測できない場合は、
理由に `signal unknown` と記録します。

`pre-push` hook は、ワークスペースをコンパイルしない検査だけを実行します
（`cargo fmt --all -- --check`、Markdownlint、SKILL.md frontmatter の検証）。
Git hook は `gwtd` ではなく `git push` の配下で動くため verification lease を
取得できず、そこで重量級の Cargo ジョブを起動すると、別の worktree が lease を
保持している間にホストを飽和させてしまいます。Clippy・テスト・カバレッジ 90%
閾値は、代わりに Lint / Test / Coverage workflow が pull request ごとに強制
します。

**移行方法:** `verify.lease.acquire`、`verify.lease.hold`、
`verify.lease.extend` は holder や予約を作らずエラーを返すようになりました。
canonical 検証を囲む手動取得は `verify.run` に置き換え、通常の Cargo 操作を
囲む手動取得は削除してください。既存の旧 holder はプロセスを kill せず、
所有プロジェクトから明示的に解放できます。

```bash
gwtd <<'JSON'
{"schema_version":1,"operation":"verify.lease.release","params":{"lease_id":"<lease-id>"}}
JSON
```

lease の遷移は
`~/.gwt/runtime/verification-coordinator/lease-events.jsonl` に記録されます。
検証は専用の coordinator レーンを持ちます。semantic search と index build は
従来どおり `~/.gwt/runtime/index-coordinator` 上で相互排他（model を load する
runner は同時に 1 本）し、検証とは互いに待ち合いません。

### PR HEAD の検証

PR の対象ブランチを訂正するには、`pr.edit` に `params.number` と `params.base`
（例: `develop`）を渡します。base だけの更新も可能で、既存の編集権限チェックを
適用します。誤った PR を取り下げるには、`pr.close` に `params.number` と任意の
`params.comment` を渡します。コメントを指定すると閉鎖前に記録し、記録に失敗した
場合は PR を開いたままにします。close はブランチを保持し、実装変更の権限や
検証証跡がない状態でも利用できます。

`pr.head_check` に `params.base`（例: `develop`）と任意の `params.head` を渡すと、
PR の作成・編集をせずに、正本の PASS 済み検証記録と live remote HEAD を比較できます。
JSON の診断には record ID、検証済み/remote/base の SHA、product commit/file、local
検証の freshness が含まれます。base 同期のみと判定されても stale な証跡は更新しません。
記録が欠落・未完了・失敗・破損している場合は unprovable を返します。

Ready PR の作成前に、`pr.create` は live remote branch と `verify.run` に記録された
HEAD を比較します。応答と PR 本文には、両方の SHA、base の SHA、比較結果が残ります。
`.gwt/` 内の bookkeeping と base 同期のみの先行は許可します。検証済み履歴と対象
base に含まれない commit が product file を変更していれば、後で revert されていても
拒否します。merge による追加の source 変更も拒否します。remote 履歴を取得できない
場合や比較を証明できない場合は、Ready を許可しません。

拒否文は product commit と file を列挙します。記載された remote branch を fetch し、
`git merge --ff-only <remote-head-sha>` で local branch を FF した後、変更範囲に対応する
検証行列を `verify.plan` に登録し、`verify.run` で再検証してください。PASS 後に
`pr.create` を再試行します。local が分岐している場合は FF の前に履歴を整合させます。

既存 PR の `pr.view` は、本文に保持された検証 SHA と現在の remote HEAD を比較し、
drift を報告します。現在の branch に対応する旧 PR は PASS 済み local 検証記録を
参照し、証跡がなければ unknown を表示します。診断を永続化するには、比較結果を
`pr.comment`（`params.number` と
`params.body`）または owner Issue の `issue.comment` に記録します。PR の表示だけでは
本文を変更せず、現在の HEAD が検証済みであるとも認定しません。

### GitHub API 予算

gwt が発行する `gh` 呼び出しは、全マシン・全 worktree・全エージェントで
1 つの GitHub アカウント予算を共有します。`pr.list` の inventory は
cache-first で、`~/.gwt/projects/<hash>/pr-inventory-cache.json` の
スナップショットが 5 分間は GitHub に触れずに応答します。一括クエリは軽量で、
`statusCheckRollup` / `body` は変更のあった PR だけ個別に取得します。判断に
ライブ状態が必要なときだけ `params.refresh:true` を渡し、重いフィールドは
`params.include`（`["checks","body"]`、既定は `["checks"]`）で選びます。応答には
`source` / `cache_age_secs` / `throttled` / `github_calls` に加え、
`hydrated`（個別取得に成功したPR数）と `skipped_unchanged`（ライブ読み取りで変更なしと判定したPR数、キャッシュ応答では0）が含まれ、予算が
予備域を下回ると最後のスナップショットが返り `throttled` に理由が入ります。

変更のない Draft / CI 未起動 PR の空チェック結果は、スナップショットの期限後も再利用します。
`updatedAt` または head commit が変わると再取得し、実行中のチェックは既定で10分ごとに再取得します。
個別取得は同時最大5件、1回の読み取りで最大30件です。`~/.gwt/config.toml` で
それぞれの間隔を独立して設定できます（0を指定するとその待ち時間を無効にします）。

```toml
[pr_inventory]
cache_ttl_secs = 300
checks_refresh_secs = 600
```

予算の観測は無料エンドポイントで行います:

```bash
gwtd <<'JSON'
{"schema_version":1,"operation":"github.budget","params":{}}
JSON
```

応答には GitHub が報告する primary window（`graphql` / `core`）、分あたりの
secondary limit のローカル推定（GitHub は公開しないため、このマシンの
`~/.gwt/github-budget/` の spawn ledger から近似）、最新の rate-limit 拒否、
そして定期読み取りが今受ける間引き判定が含まれます。

### リリース手順

リリースは GitHub Actions の **Prepare Release** ワークフロー（Actions →
`Prepare Release` → `Run workflow`）で起動します。CI が `develop` を対象に
バージョン更新・`CHANGELOG` 再生成後、その develop commit を固定した
`release/vX.Y.Z → main` の Release PR を作成します。以降 develop へ着地しても
release head とその CI は変わりません。ローカルで `develop` に切り替えずにどのブランチからでも
リリースできます。`bump` 入力は `auto`（既定）/ `patch` / `minor` / `major`。
`auto` がメジャーになることはありません。コミットの breaking marker は
Release PR 本文に列挙されるだけで、メジャー昇格は `major` を明示した場合のみです。
生成された Release PR をレビューしてマージすると、`main` 側でリリース
パイプライン（タグ・GitHub Release・各プラットフォームのバイナリ）が走り
ます。リリース復旧手順は `.claude/commands/release.md` にあります。

Release PR の本文は参照専用です。配信した Issue は裸の `#N` 参照で列挙し、
closing keyword は書きません。`main` は default branch なので、そこに
`Closes #N` があると受け入れ基準が未消化の Issue まで閉じてしまうためです。
Issue の決着は work ブランチが `develop` に merge された時点で行われます
（前述）。merge 後は `release.yml` が `scripts/release_close_guard.py` を実行し、
Release PR の merge 自体が閉じた Issue を reopen してマーカー付きコメントを残します。

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
JSON operation `issue.spec.read` を使って確認できます。

## ライセンス

MIT
