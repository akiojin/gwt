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
  custom agent を共有 workspace から起動・再開・状態確認できます。
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
- **Operator workspace** — レールから Issues、Agents、Board、Settings を切り替え、
  必要に応じて Split view と独立した PM 入口を使えます。
- **Provider usage** — USAGE にホバーすると、Claude の Fable weekly 枠や、
  有効な Usage credits の使用率・使用量/上限・通貨を確認できます。
  Claude アカウント枠は Settings → Usage & Limits で有効化してください。

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

**Settings > Supported Agents** では、全対応エージェントの導入状態と
インストール済みバージョンを確認し、未導入の CLI をインストールできます。
公式配布元の情報で新しいバージョンを確認できた場合は更新操作が表示されます。
Codex・Grok・OpenCode・OpenClaw は npm、Antigravity は platform manifest を照会します。
Claude native と Hermes は実効更新チャネルを検証できないため、理由を表示し、
更新は手動で行います。`gh copilot` も、検出した配布物と extension updater の対象が
一致すると保証できないため手動更新とします。取得に失敗した場合も理由を表示して更新操作を無効にします。

エージェントの自動更新は既定で無効です。有効にすると、次回の gwt 起動時に
更新経路を検証できるインストール済みエージェントを確認し、新しいバージョンが分かるものだけを
更新します。PM を含む稼働中のエージェント窓や起動待ちがない場合に限り適用します。
手動のインストール・更新でも、先に稼働中のエージェントを閉じてください。
未導入のエージェントを自動でインストールすることはありません。

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
- **Open Project…**: Hub を開き、フォルダ選択を直接開始します。
- **Clone from GitHub…**: Clone Project ダイアログを開いた状態で Hub を表示します。
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
表示します。Hub はトレイの **Open in browser**、または引数なしの `gwt open` で開けます。

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

Lifecycle: 起動中の `gwt` プロセスが agent / PTY の寿命を所有します。既定のトレイ常駐
モードでは、ブラウザのタブを閉じてもプロジェクトと実行中の agent は継続し、
プロジェクト URL を開き直すと同じワークスペースに戻れます。プロジェクトを停止するには、そのページのヘッダーか
Hub の開いているプロジェクト一覧にある **Close Project** を選びます。実行中の agent が
ある場合は、対象を列挙した確認ダイアログが表示されます。**Cancel** なら実行を継続し、
**Close Project** ならそのプロジェクトの runtime と agent を停止します。他のプロジェクトは
継続し、閉じたプロジェクトを表示していたすべてのブラウザタブは Hub に戻ります。

サーバー全体を停止するには、トレイの **Quit**、または `Ctrl-C` / `SIGTERM` を使います。
gwt は PTY をドレインしてから graceful shutdown を実行します。tray 常駐プロセスは
1 OS-login ユーザーにつき 1 つだけで、二重起動した `gwt` は既存 URL を出して終了します。

`gwtd` operation は stdin JSON envelope で処理され、gwt のメインウィンドウは起動しません。
承認が必要な operation では OS のネイティブダイアログが表示されることがあります。

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

`fleet.inventory` はプロジェクトの agent プロセス・pane・Monitor 行を1つの
JSONレポートで返します。対応が取れない行、worktreeの重複、配信の判定根拠、
一部の観測元が使えない場合の診断も含みます。`fleet.orphans` はpaneが観測されない
プロセスを列挙します。pane情報の取得失敗時はorphanと断定しません。

登録済みPMは `fleet.disposition` に空でない `targets` 配列を渡して整理できます。
選んだ行の `id`、`session_id`、paneの `child_pid` / `child_started_at` を指定します。
プロセスのみの対象には代わりに `pid` / `started_at` が必要です。
配信状態がunknownの集合は `allow_unknown:true` を明示しない限り拒否されます。
現在のclaim保持者、稼働中executionの保持者、未配信の作業は保護されます。
`fleet.orphan.terminate` はプロセス対象のみ受け付けます。対象ごとにreceiptを返し、
世代変更、未対応backend、プロセス終端未確認を報告します。通常closeへはfallbackしません。
orphan終端には固定OS process handle（LinuxのpidfdまたはWindowsのprocess handle）が必要です。
他のOS、handle取得不可、lifecycle権限が不確かな場合はプロセスを保持します。
`allow_unknown` で上書きできるのは配信状態の不確かさだけです。

`pane.health` は既存の workspace と runtime sidecar の状態から、GUI event loop に
依存せずプロジェクトの pane health を読み取ります。応答の `output` 文字列を JSON として
読んだ `panes` 配列に、各 pane の `classification`、`reason_code`、`recommended_action`
を返します。`params.id` を省略すると全 pane、`pane.list` の ID（raw pane ID も可）を
渡すと単一 pane が対象です。
推奨 action は判断材料であり、この operation 自体は close・relaunch・stop・requeue
を実行しません。

```bash
gwtd <<'JSON'
{"schema_version":1,"operation":"pane.health","params":{}}
JSON
```

`actions.dispatch` は共有 GitHub API budget を使って workflow を起動します。
既定ではすべて拒否します。`~/.gwt/config.toml` でリポジトリごとに
workflow のファイル名を許可します。

```toml
[allowed_workflows]
"akiojin/gwt" = ["prepare-release.yml"]
```

```bash
gwtd <<'JSON'
{"schema_version":1,"operation":"actions.dispatch","params":{"workflow":"prepare-release.yml","ref":"develop","inputs":{"bump":"auto"}}}
JSON
```

`workflow` はファイル名または正の workflow ID を受け付けます。ID の場合も
解決したファイル名が許可されている必要があります。checkout の
`.github/workflows/` にファイルが存在し、指定した branch / tag 上で
`workflow_dispatch` を宣言していることを検査します。`inputs` は省略可能な
JSON object です。入力名、必須値、型、choice の選択肢を送信前に検査します。
成功応答は送信の受付を示します。実際の run は GitHub Actions で確認してください。
拒否時は修正すべき設定や入力契約を示し、入力値や GitHub の生エラー本文は出力しません。

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
3. **Curate** レーンでは、Command Palette の `Intake` を
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
  Work Item view を開きます。Issue Monitor が自動起動した Agent は、
  Settings の Issues 右ペインに読み取り専用でミラー表示されます。入力
  したいときは `Windowize` で同じセッションを Agents の個別タブに開けます。各行には対応する Work
  の lifecycle・注意理由・PR 状態が表示され、`Continue work` / `Resume` /
  `Clean Up` をその場で実行できます
- `Logs` — project diagnostics と live log surface
- `Profile` — environment/profile 管理
- `Branches` — branch 確認、filter、cleanup、Git detail
- `Settings` — application と agent の設定。`System` タブで Workspace summary
  と Board 投稿本文の出力言語を `Auto / English / 日本語` から選択できます
  (Auto は OS locale を参照し、`C` / `POSIX` や未設定時は English にフォール
  バック)。設定はグローバルで `~/.gwt/config.toml` の `[ai].language` に保存
  されます。UI 文言は引き続き英語固定です (SPEC-1933 NFR-005)。
  アプリ全体とプロジェクトごとの設定は区別して表示します。`Issues` では
  Autonomous、Max active、キュー補充、許可ラベル、起動プロファイルと候補の
  順序・優先条件を同じ欄で編集できます。
  起動候補の変更は確定時に自動保存され、テキスト欄はフォーカスを外すと保存されます。
  拒否された入力は保持され、該当欄の近くにエラーを表示します。
  Settings は Application / Project の左ナビと右の内容ペインに分かれ、編集対象の名前を表示します。
  `System > Auto-apply updates` は全プロジェクト共通で、開いて
  いるプロジェクトの少なくとも1つで Autonomous が有効なときに適用します。
  旧プロジェクトの値は一度だけ移行し、どれか1つが ON なら全体を ON にします。
  System には保存値・有効値と、更新の保留理由も表示します。GUI で変更するか、
  PM に `update.settings.set` の `auto_apply_updates` で変更を依頼できます。
  `update.settings.status` で状態を読み取れます。
  この設定は Issue Monitor の設定から除外されています。
  `Restart now` では稼働中 agent の件数、pending claim、verification lease、
  関連 Issue を確認してから適用します。作業中は `Apply after all agents finish`
  が既定の選択肢となり、新規起動を保留して既存作業の終了まで待ちます。
  代わりに `Restart now` を明示確認すると、稼働中 agent を停止して直ちに再起動します。

`Agent` は coding agent セッション用の実プロセスウィンドウです。`Board` は
Agent が status、decision、handoff、request を外部化する coordination surface です。
Work Item Knowledge Bridge は frontend から GitHub API response を直接描画せず、
ローカル Issue cache と semantic index を使います。

Board は各ウィンドウで最大1,000件を保持します。先頭までスクロールするか
`Load earlier entries` を選ぶと、古い投稿を再取得できます。過去の履歴を閲覧中は
`Back to latest entries` で最新の更新へ戻れます。

gwt は作業 Agent の起動時に、出力言語に合う覚えやすい名前をランダムに割り当てます。
バッジ・ウィンドウタイトル・Fleet・Kanban・Board に表示し、PM も名前で状況を報告します。
名前はプロジェクトの過去の担当名と重複せず、番号の接尾辞は付けません。
候補を使い切ると言語に合う姓を組み合わせて候補を広げます。
自動復元では既存の名前を維持し、Issue を担当する Agent の明示的な再起動では新しい名前を割り当てます。
担当開始・引継ぎ・PR 作成・完了は担当名付きで Issue のコメントに残り、PR 本文にも担当名を載せます。
常駐 PM は独自の固定表示を維持します。

Windows の Host 起動では、Launch Agent で Command Prompt、Windows PowerShell、
PowerShell 7 を選択できます。Docker 起動では引き続きコンテナ内のシェルを使います。

ターミナルウィンドウでは、テキストをドラッグ選択してマウスボタンを離すとコピー
できます。Windows では `Ctrl+C` で現在の選択をコピーして選択を解除します。
選択がない場合、`Ctrl+C` は実行中のターミナルプロセス向けの割り込みのままです。
Linux では `Ctrl+Shift+C` でも現在の選択をコピーできます。

## Issue サーフェスと Issue Monitor

**Settings → Project → Issues**、または左ナビの **Issues** から、キャッシュ済み GitHub Issue を Backlog / Queued /
Active / Done の 4 列で表示します。各行の実行状態と操作はそのまま利用できます。
Backlog から Queued へドラッグすると実行対象に追加され、逆方向でキューから外れます。
Queued 内では順序を変更でき、複数選択の移動は 1 回のキュー操作で送信されます。
Active と Done は実行ライフサイクルに従うため、ドラッグでは変更できません。
Queued には `auto-refill` などの追加元も表示されます。

検索と Kanban / Split の切り替えは同じ行に並びます。要約帯には Running（稼働中）、
Waiting（Full / Hold / Empty）、Stopped（停止）、Error の状態、Active / Queue の件数、
容量通知へのリンク、Autonomous、Start / Stop、⚙ を表示します。現在の障害だけを赤く表示し、
エラーの詳細と履歴は通知センターで確認できます。⚙ で同じペインの設定行を展開し、
**Launch control（起動制御）**で Autonomous・Max active・Start / Stop、**Refill（補充）**で Auto-refill と上限、
**Target（対象）**で Allowed labels を設定します。その下に候補プロファイルが並びます。
設定を閉じると Autonomous と Start / Stop は要約帯に戻ります。Auto-apply updates は **System** で設定します。
Max active は **Manual** で数値を編集し、**Apply** で最終値を保存します。
**Auto** では編集できず、実効上限を表示し、計測が未完了なら **Measuring capacity…** と表示します。
稼働数より小さい上限に下げても既存エージェントは継続し、稼働数が上限未満になるまで新規起動を待ちます。
Auto-refill は**既定で OFF**です。
有効にすると、条件を満たす open Issue を設定したキュー上限まで自動補充します。
空のキューから新しい作業は起動せず、実行中の作業は継続します。

Kanban は既定で全幅です。カードを選ぶとプレビューが開き、**Issue / Output** で
本文・受け入れ基準とエージェントの読み取り専用出力を切り替えます。
**Windowize** でエージェントを Agents の個別タブに開けます。**Close preview** または Escape で
全幅に戻ります。狭い幅ではプレビューをボードの上に重ね、列は縮めず横スクロールします。
従来の `issue_monitor` preset も同じ Issue サーフェスを開きます。

**Max active** は新規設定で **Auto** を使用します。推奨値には CPU、空きメモリとディスク、
GUI の CPU 使用量、他プロジェクトの稼働中エージェントを反映します。稼働中のエージェントが
ない登録済みプロジェクトは配分を消費しません。**Machine budget** には制約になった資源を
表示し、Monitor の実装・レビュー上限と PM を含む総数を区別します。計測が未完了または
期限切れの間、Auto は直前の有効上限または保存済み上限（新規設定では 1）を保持します。
`target` が存在しない場合や計測中の場合も、それを理由に新規起動を止めません。
完全な実測により上限が 0 になることはあります。容量を下げた資源や稼働中エージェントの
消費は status とエラー台帳に記録します。
**Manual** で正の数値を入力して **Apply** を押すと固定上限を保存し、**Auto** で推奨値への追従に戻せます。
Auto / Manual と手動の要求上限は、このマシンの全プロジェクトで共通です。Manual は
実装・レビュー窓の合計上限で、他プロジェクトの消費を引いた残りを各プロジェクトへ配分します。
PM 窓の会計は別に表示し、Settings には自プロジェクトの割当と他プロジェクトの消費を併記します。
共通の容量設定の仕様は [SPEC #3200](https://github.com/akiojin/gwt/issues/3200) を参照してください。推奨値を超える入力も許可しますが、
検証が完走しない可能性と、時間に依存するテストの失敗が無関係な PR を妨げる可能性を警告します。
自動化では `issue.monitor.config.set` に `{"max_active_mode":"auto"}` を渡すと Auto、
`{"max_active":4}` を渡すと上限 4 の Manual になります。`issue.monitor.status` は実効上限、
`max_active_agents_override`（共有の要求値）、`max_active_agents_scope: "machine"`、共有実測値の `agent_capacity` を返します。
`max_active_last_change` は最後の明示的な設定変更の `changed_at` と
`source`（`gui` / `json_operation`）を返し、自動計測による上限更新ではこの記録を保持します。

**Settings → Issues** の **Allowed labels** にラベルをカンマ区切りで指定し、保存済みリストのいずれかに一致する Issue が対象になります。
大文字・小文字と前後の空白は区別しません。空のリストは全ラベルを許可し、従来の対象条件を
維持します。変更は次の scan で反映され、実行中のエージェントは中止しません。
自動化からは
`issue.monitor.config.set` に `{"allowed_labels":["agent:mac"]}` を渡せます。
`issue.monitor.status` は `allowed_labels`、`label_excluded_count`、
`label_excluded_issues` を返します。

open な GitHub Issue は、明示的な追加、有効な Auto-refill、または `urgent` ラベルによる投入
まで Backlog に留まります。キューへの所属は Monitor の実行候補になる条件であり、
準備状態・claim・同時実行数のチェックは引き続き適用されます。行の `Launch now` は
起動フローを開き、起動時に `work/issue-N` のブランチ/worktree を作成して
`gwt-execute #N` でエージェントを開始します。起動失敗は Issue 行に残ります。
live な claim/window の識別情報が失われた場合も、operator は `issue.monitor.stop` に
`params.number` と `params.reason` を渡して失敗した launch を停止できます。
保存済みの失敗 window は同じ Issue の所有である場合だけ使用し、永続的な停止状態と
閉じる対象の pane を返します。
停止した Issue は理由付きの保留として表示され、既定ではこの端末の起動キューから
外れます。`params.remove_from_queue` を `false` にすると、保留したままキューへの
所属を維持します。起動失敗には原因の分類と次の回復操作を表示し、同じ原因の失敗が
続いた場合は設定済みの試行上限後に自動再試行を止めます。

live な launch の停止では、pane が消えていても、その host に対象の agent と子孫
プロセスの終了を要求します。`runtime_stop: requested` は要求の受付を示すため、
終了は `issue.monitor.status` で確認してください。`active_sessions` は pane の有無に
関係なく生存 PID・worktree・既知の `launch_entrypoint` を返します。
停止した runtime は検証を開始できません。既存の作業を続けるには、Session を明示的に
再開して新しい runtime を起動します。対象を安全に特定できない場合は、停止を拒否します。
別の pane に対する `pane.close` は、プロセスツリーの終了後に `closed` を返します。
タイムアウト時や、復元した Session の PTY を捕捉できない場合は終了未確認のエラーと
なります。自分の pane を閉じる場合は、呼び出した agent が終了する前に
`close requested` を返します。

`urgent` は誰でも付与できます。実行候補の urgent Issue は Auto-refill が無効でも
自動投入されますが、明示的なキュー削除は優先されます。既定では付与順に最大2件が
先頭群へ入り、保存済みの通常順序と `max_active` は変わりません。
`issue.monitor.queue.urgent_limit` の `limit` で上限を変更できます（`0` は先頭群への昇格だけを無効化）。
変更前に `issue.monitor.status` の `saved_config` と `config_token` を読み、
その token を `expected_config_token` として渡します。保存観測値には
`urgent_queue_limit` を含みます。不一致なら設定を保存せず、status を再読して変更を再評価します。
応答は従来のキュー項目を保持し、同じ保存 snapshot の `saved_config` と `config_token` を含みます。
guard 省略時は従来の挙動を保ちます。この値比較は ABA 履歴を検知しません。
超過分は通常順序に従います。`issue.monitor.queue.demote` の `number` で指定した Issue は
永続的に通常優先度へ戻り、scan・再起動後も urgent ラベルより降格が優先されます。
降格前に `issue.monitor.queue.list` を読み、`urgent_queue_token` を
`expected_urgent_queue_token` として渡します。保存された urgent 上限・付与情報・降格を
比較対象とし、通常のキュー順序と runtime は含みません。不一致なら設定を変更せず、
list を再読して降格を再評価します。成功応答のキュー項目と新 token は同じ保存 snapshot を表します。
guard 省略時は従来互換で、`null`・空文字・異なる型は拒否します。
この値比較は ABA 履歴や他の control 操作を保護しません。
カードと詳細では urgent・上限超過・降格を区別し、`issue.monitor.queue.list` と
`issue.monitor.status` からも理由と観測済みの GitHub ラベル付与者・時刻を確認できます。
未取得の履歴は不明として表示します。

Agent や自動化からは `issue.monitor.status` で確認し、`issue.monitor.queue.push`、
`issue.monitor.queue.remove`、`issue.monitor.queue.move` で所属と順序を変更できます。
`issue.monitor.queue.list` は、選択端末の保存順 `queue_order` を urgent の表示順と別に返します。
move/remove の前に現在端末の list を読み、その配列を `expected_queue_order` として渡します。
不一致なら設定を変更せず拒否するため、list を再読して変更を再判断し、成功後も読み戻します。
明示的な `[]` は保存キューが空の場合だけ一致し、`null` は拒否します。省略時は従来互換です。
比較対象はキュー順の値であり、途中の変更履歴、urgent metadata、優先順だけの変更、他の設定変更は検出しません。
`issue.monitor.queue.auto_refill` は自動補充の有効化と上限を設定します。
変更前に `issue.monitor.status` の `saved_config` と `config_token` を読み、
その token を `expected_config_token` として渡します。保存観測値には
`terminal_queue_auto_refill` と `terminal_queue_auto_refill_limit` を含みます。
不一致なら設定を変更せず拒否するため、status を再読して変更を再判断します。
成功応答の保存観測値と新 token も読み戻して確認します。省略時は従来互換で、
`null`・空文字・異なる型は拒否します。途中の変更履歴や他の control 操作は保護しません。
`issue.monitor.launch_now` は対象を端末キューの先頭へ明示的に追加し、scan を要求します。
既存の `issue.monitor.priority.move` と `issue.monitor.priority.set` も利用できます。
途中の優先順変更を上書きしないため、`issue.monitor.status` の保存済み全順序
`priority_order` を読み、いずれの優先順操作または `issue.monitor.launch_now` にも同じ配列を
`expected_priority_order` として渡します。不一致ならキュー追加・hold 解除・scan 要求の前に
更新を拒否するため、status を再読して指示を再判断し、成功後も
読み戻して確認します。明示的な `[]` は保存順序が空の場合だけ一致し、`null` は拒否します。
引数を省略すると従来の挙動を維持します。比較対象は現在の順序の値であり、途中の変更履歴や
他の設定変更は検出しません。
`issue.monitor.config.set` は処理停止、Autonomous
モード無効化、正の `max_active` 上限設定に対応します。条件付きで設定を変更する場合は、
`issue.monitor.status` の `saved_config` と `config_token` を読み、同じトークンを
`config.set` の `expected_config_token` として渡します。不一致なら設定を保存せず、
scan も要求しません。status を再読して変更を再判断し、成功後も読み戻して確認します。
比較対象は保存設定の値で、runtime の観測値や途中の変更履歴は対象外です。
省略時は従来互換、`null` は拒否します。旧 daemon はガードを外して適用せず、
ガード付き変更を拒否します。リポジトリの登録済み PM は
`enabled` / `autonomous_mode` の有効化もできます。他のエージェントは GUI から有効化します。
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
プールを置き換えます。両応答は opaque な `profiles_token` を返します。
更新時は直前に読み取った token を `expected_profiles_token` に渡します。競合時は
preferences を変更せず拒否するので、profiles を再読して変更を再評価し、成功後も
読み戻してください。token の対象は保存プールと継承元の先頭候補であり、他の設定、
runtime 状態、ABA 履歴は対象外です。guard を省略した既存呼び出しは互換です。
候補が 2 件以上あると、Monitor は各 Issue を最初の適格な候補
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

Monitor が自律起動した Codex が作業途中に「Selected model is at capacity」で
ターン終了した場合、gwt は60秒以上待ってから、同じ稼働中 Session に続行指示を
一度だけ送ります。同じ gwt インスタンスの稼働中、自動送信は Session ごとに最大一回です。
回復、手動入力、承認・quota
待ち、Session の置換・精算を検出した場合は送りません。再失敗時も既存の PM 通知を
保持します。手動起動した Session の続行は操作者が行います。

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
条件付きで `tiers.set` を実行する場合は、`issue.monitor.status` または
`issue.monitor.tiers` の fresh な `saved_config` と `config_token` を読み、
その token を `expected_config_token` として渡します。不一致なら設定ファイル全体を
変更せず拒否するため、status を再読して変更を再判断し、成功後も読み戻して確認します。
token は保存された `launch_auto` と `launch_tiers` を含みます。guard の省略は従来互換を保ち、
null・空白・文字列以外は拒否します。値の比較であり、ABA 履歴は検知しません。
条件付きの `tier.set` にも、同じ fresh な設定 token を `expected_config_token` として渡せます。
保存設定が変わった後の tier 番号の適用を防ぎます。競合時は設定ファイル全体を変更せず拒否するため、
status を再読して変更を再判断し、成功後も読み戻して確認します。guard の検証と省略時の互換性は同じです。
これは保存設定の比較であり、個別 override の履歴や ABA 履歴は検知しません。
既存の段の下限を下げない更新は維持します。
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
JSON の承認フラグや過去の承認は利用できません。独立した Agent タブで動く、直接インストールした
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
受け入れ基準がすべてチェック済みなら、PR 番号と merge
SHA を含むコメントを投稿して Issue を close します。未達の基準が残る場合は
`merge 済み・未達 AC あり` コメントを残して `NeedsHuman` にし、`gwt-spec` Issue は
全 Phase の tasks が完了したときだけ close します。残りの基準を別 Issue に
委譲しても、未チェックの項目があれば close しません。auto-close は既定で
`Autonomous` トグルに連動し、`issue.monitor.config.set` の
`auto_close_merged_issues=true|false` で上書きできます（off のときは
`merge 済み・close 待ち` コメントの記録のみ）。人間が reopen した Issue を同じ
merge で再度 close することはありません。

完了として close 済みでも未チェックの受け入れ基準が残る Issue は、
この checkout で `node scripts/audit-closed-acceptance.mjs OWNER/REPOSITORY` を
実行すると列挙できます。読み取り専用の監査が `gh` で閉じた Issue の全ページを
取得し、Issue 番号・URL・未チェック基準の JSON 一覧を出力します。PR と
「対応しない」として閉じた Issue は除外します。コメント保存の SPEC は参照先の
`spec` コメント（分割 section を含む）を読み、読めない section は黙って飛ばさず
エラーとして報告します。

無人運転中のライフサイクルイベント（マージ完了・再試行予約・ゲート通過・
NeedsHuman エスカレーション）はトーストとして表示され、永続的なスクロール可能
通知スタックに蓄積されるため、離席中のイベントも失われません。

Agent の状態通知は **停止**・**エラー**・**人間の対応待ち**を伝えます。
Idle を Work 完了とは扱いません。runtime のデスクトップ通知は、同じ runtime 世代で
連続した Running をサーバーが5分以上観測した場合に限ります。全ブラウザタブを
閉じても計測を続け、別状態への遷移や runtime 世代の変更でリセットします。Monitor の NeedsHuman は窓のない
Issue も既存の通知ストリームで即時に伝え、inbox snapshot から二重に通知しません。
Session Interrupted は再開候補の過去 snapshot としてしか公開されていないため対象外です。

ネイティブ権限の adapter は macOS の認可設定、Windows の通知設定、Linux の
認可照会不可を区別します。不明・未設定・拒否・照会失敗では配送を許可せず、権限を
自動要求しません。Linux の GetCapabilities はサーバー機能であり、ユーザーの許可では
ありません。同じ Project に表示中かつフォーカス中のタブがあればデスクトップ配送を
抑止し、それ以外では許可済みタブのうち最後に注視情報を受信した1つへ配送します。
Project の接続タブが0個の場合、最後に報告されたブラウザの許可と GWT 自身の現在の
OS 許可があればネイティブ配送できます。未設定・拒否の報告は保持した許可を取り消し、
Project の明示終了や GWT 再起動では許可を破棄します。クリックは受信タブへフォーカスするか、
安定した Project URL を開きます。配送失敗時は別経路へ再送せず、未読・title・favicon・
トレイ更新は継続します。macOS には許可済みの署名済み GWT.app、Windows には MSI が
登録する AUMID 付きショートカットが必要です（portable ZIP は登録しません）。Linux は
認可照会ができないためネイティブ配送を抑止します。debug binary では判定とブラウザ挙動を
検証でき、署名済み macOS／インストール済み Windows の配送・クリック操作は別途実機検証が必要です。

調整可能な上限（試行回数・stuck/idle タイムアウト・再試行バックオフ・レビュー
モデル）はプロジェクト単位で永続化されます。human-gated の基礎は SPEC
[#3165](https://github.com/akiojin/gwt/issues/3165) を参照してください。

## PM エージェント

各プロジェクトには常駐の **PM エージェント**ペインが 1 つ起動します。これが
ユーザーの唯一の対話窓口です。自然言語で要望を伝えると、PM が Issue への分解・
登録・design-required Issue の計画策定・意味的な実行順序の決定・Issue Monitor
への起動指示までを行います。進捗の報告と `NeedsHuman` エスカレーションの提示も
同じ会話の中で行われます。

PM が要望の調査などを委譲する補助エージェントは、イベント購読を委譲する場合も含めて
同時 1 件までです。追加の委譲は完了後に行い、次の購読よりも受付済みの要望を優先します。
その間も PM は対話と状態スナップショットの確認を続け、harness 側の上限にも従います。
harness の空き枠が不明な場合は、自身の補助枠が空いているときに限り起動を 1 回試し、
拒否された作業は連続再試行せず保留します。

計画中の Issue をプロジェクト目標、または明示的な maintenance / compliance
区分へ紐付けるよう PM に依頼できます。`pm.plan.status` は保存済み計画の revision と、
未紐付け・不正な紐付け、Now/Next/Later、マイルストーン情報の不足を示す `policy`
診断を返します。Monitor の観測には取得元とスキャン時刻が付き、取得不能・不完全な
観測から全件の確認済みとは判断しません。記入済みの health / confidence も、裏付け
証拠が接続されるまでは検証済みとして扱いません。
`policy.roadmap_items` の各行は、その項目に直接保存された `issues` を順序・重複を
保持して返します。`observed_issue_states` と `unobserved_issues` は Issue 番号の
昇順・重複なしで、typed state は供給された inbox 順のまま重複を除きます。
空・重複 ID や不正な紐付けも行を分けて保持し、Goal・Milestone・他の行から所属を
広げません。queue/active 所属だけでは inbox 状態を推定しません。Monitor がなければ
直接所属の全 Issue を未観測とし、degraded 観測では取得元を保持します。空所属は
空配列です。この観測だけで health・完了・優先順位・起動許可を判断しません。
マイルストーンの `target_date_observation` は、厳密な `YYYY-MM-DD` の目標日と、
記録済みスキャンの UTC 暦日を比較します。`days_past_target_at_scan` は目標日の当日・
以前なら 0、日付が欠落・不正なら不足情報名とともに不明を返し、現在時計を使いません。
取得元とスキャンの古さも併せて確認してください。目標日からの経過だけでは、未完了の遅延・
health・完了を証明しません。
`policy.review_dates` は保存されたリスク、次に意思決定を、保存順・ID・記入済みの
レビュー日を保持して返します。`days_past_review_at_scan` は厳密な `YYYY-MM-DD` と
スキャンの UTC 暦日を比較し、レビュー日の当日・以前なら 0 を返します。日付が欠落・
不正なら不足情報名とともに不明になります。現在時計を使わず、取得元・スキャンの古さ・
観測範囲の制約も引き継ぎます。レビュー日からの経過は、レビュー実施状況の証明や
操作の許可にはなりません。
`policy.risks` は保存順のリスク ID と、空または空白のみの `id`・`severity`・
`probability`・`impact`・`owner` を示す `missing_inputs` を返します。
非空 ID の完全一致重複には `unique_risk_id` を追加し、大文字小文字と前後の空白は
ID の区別に保持します。入力済みの値の意味は評価せず、assumptions は任意です。
この診断は Monitor の取得を必要とせず、既存のレビュー日観測を維持します。
情報が揃っていても、リスク評価の妥当性や操作の許可を示しません。
`policy.decisions` は保存順の意思決定 ID と、空または空白のみの `id`・`rationale`・
`owner` を示す `missing_inputs` を返します。`options` に空白以外の候補が一つも
なければ `options` を示します。非空 ID の完全一致重複には `unique_decision_id` を追加し、
大文字小文字と前後の空白は ID の区別に保持します。入力済みの値の意味は評価せず、
assumptions と `source_revision` は任意、レビュー日は既存の `policy.review_dates` に任せます。
この診断は Monitor の取得を必要としません。情報が揃っていても、判断の品質・承認・
操作の許可を示しません。
`policy.forecasts` は保存済み予測を保存順で返し、ID・`source_revision`・`sample_size`・
`reported_estimate`・`reported_confidence`・`reported_missing_inputs` を保持します。
`revisions_since_source` は `planning_revision` との非負差分で、未来の参照値なら null です。
参照 revision の 0 は有効です。`missing_inputs` は常に `forecast_evidence` を含み、続いて
空白 ID の `id` または非空 ID の完全一致重複の `unique_forecast_id`、標本数 0 の
`sample_history`、未来参照の `source_revision_ahead_of_planning` をこの順で示します。
非空 ID の大小文字・前後空白は区別し、記入済み不足情報は原文のまま保持します。
診断の `confidence` は `unknown` のままです。正の標本数・同値 revision・記入済み信頼度は、
検証済み履歴・予測品質・承認・操作許可を証明しません。小標本の閾値や予測値を推測しません。
Monitor の取得には依存せず、保存予測がなければ `[]` を返します。
`policy.decision_revisions` は全保存意思決定の ID と `source_revision` を保存順で返します。
`revisions_since_source` は現在の `planning_revision` との差分で、0を含む非負の数値です。
参照 revision が未設定なら差分は null、`missing_inputs` は `source_revision`、
現在計画より先なら差分は null、`source_revision_ahead_of_planning` を返します。
Monitor の取得を必要とせず、`source_revision` の設定も任意のままです。
revision の差は判断の入力変更・鮮度・品質・承認・操作許可を証明しません。
`policy.recurring_runs` は保存済みの定期作業を、保存順・ID・記入済み `next_run` を
保持して返します。`due_at_scan` は時差・秒未満の精度を含む RFC3339 の絶対時刻を比較し、
スキャンが `next_run` と同時刻以降なら true、前なら false を返します。時刻が欠落・
不正、または小数秒が9桁を超える場合は、`next_run` / `monitor_scan_time` の不足情報名とともに不明になります。
記録されたスキャンだけを使い、取得元・古さ・観測範囲の制約を保持します。
現在時計や cadence の解釈は使いません。時刻の到来は未実行の証明や、
スケジュール・Issue 作成・起動の許可にはなりません。
各行の `configuration_missing_inputs` は、空または空白のみの `id`・`cadence`・`owner`・
`miss_policy`・`idempotency_key` を時刻診断とは別に示します。非空の ID や設定キーが重複する場合は、
一致する全行に `unique_recurring_id`・`unique_idempotency_key` を示します。比較は保存文字列の
完全一致で、空白の除去は不足判定だけに使います。Monitor の有無には依存せず、cadence の解釈・
owner の解決・policy の意味検証は行いません。空配列でも実行許可や重複防止の保証にはなりません。
各 Issue の trace は、未紐付けや maintenance の項目も `observed_inbox_states` を示し、
claim 競合・保留・失敗の説明に使えます。状態は観測順で重複を除き、inbox の行が無ければ、
queued / active でも空配列を返します。この観測で目標との整合判定を変えたり、
優先順位・実行可能性・health・完了を判断したりはしません。
容量観測では、実装・レビュー・claim 待ち・人間待ちの Issue を区別します。
slot の使用数と上限は Monitor の報告値を保持し、Issue 件数から再計算しません。
統合した Issue 一覧はキュー待ちだけの作業を除く下限集合で、全工程の WIP ではありません。
取得不能な観測は不明のまま扱い、verification と全工程 WIP の不足も示します。
この診断の読み取りで起動判断や WIP 上限の強制は行いません。
保存済み WIP policy は保存順に独立して、重複を除いた観測 Issue 件数と照合します。
`observed_exceeded` は、このスナップショットの件数が保存上限より厳密に大きいことだけを
示し、一意かつ空でない policy ID、設定済み上限、daemon の完全な観測範囲と有効なスキャン時刻を
必要とします。等値・下回る場合や情報不足・degraded では `unknown` とし、上限内 PASS は
返しません。Monitor 不在なら件数は null、policy 不在なら空リストです。全工程 WIP・現在の
超過・操作権限を証明せず、policy の選択や合成も行いません。verification と全 WIP の不足は残ります。
`policy.capacity.terminal_queue` は、urgent 順序付けとラベル除外後のホスト別 queue を、
供給された順序・Issue の重複・元の `queued_at` を保持して返します。
`age_seconds_at_scan` は queue に所属してから Monitor のスキャン時点までの秒数です。
RFC3339 の時差を反映し、秒未満を切り捨てます。時刻の欠測・不正・未来は null と不足情報名で
示し、小数秒が9桁を超える場合は黙って丸めず不明にします。Monitor または queue view が
なければ null、供給された view に行がなければ [] です。取得元・スキャン時刻・観測範囲も
併せて確認してください。古い・degraded な snapshot は当時の観測であり、[] はプロジェクト
全体の queue が空である証明ではありません。旧 payload の queue リスト省略も [] となるため、
ホスト内の空 queue も証明しません。所属滞留時間は実行待ち時間、SLA 違反、starvation、
優先順位、preemption や起動許可の判定には使いません。
idle window の観測は、Monitor の window 順序、重複・不明の Issue、idle 分類、binding と
pane の事実を保持します。idle 開始から指定のスキャン時点までの経過秒を、時差を反映し、
現在時計を使わずに返します。時刻が欠落・不正・未来なら期間は不明とし、不足情報名も示します。
Monitor 不在なら null、観測済み idle window がなければ空リストです。取得元とスキャンの古さも
併せて確認してください。キュー滞留時間・SLA・活動状態・解放の許可を証明せず、分類・slot・
claim・起動も変更しません。
活動観測は、Launching / Launched の Issue の最終活動から Monitor のスキャン時点までの
経過秒を示します。与えられた時刻だけを使い、現在時刻は参照しません。時刻が欠ける・不正・
スキャンより未来の場合は経過時間を不明とし、Monitor の取得不能は観測済みの空配列と区別して
null を返します。取得元とスキャンの古さも併せて確認してください。この値でキュー待ち時間・
SLA 違反・活動状態を判定したり、操作を起動したりはしません。
claim 競合の観測は、別の claim に保留される Issue と、スキャン時点で期限まで残る秒数を
示します。期限経過は 0 秒、時刻の欠落・不正は不明として扱います。スナップショットに
開始時刻がないため、ブロックの継続時間も不明のままです。スキャンの取得元を保持し、
この観測で claim の解除を証明したり、requeue・起動・preemption を許可したりはしません。
待機宣言の観測は、宣言を持つ Launching / Launched の Issue について、スキャン時点の
経過秒と期限までの残秒を示します。期限経過に見えても Monitor の `in_force` 値を保持し、
保護状態を再計算しません。時刻の欠落・不正は該当する期間を不明とし、スキャン後の開始から
経過時間を求めたり、開始前の期限から残時間を求めたりはしません。Monitor が取得できなければ
null、該当行がなければ空リストです。スキャンの取得元を保持し、理由や再開条件の自由文を省きます。
キュー滞留時間・SLA・活動状態・全 WIP の判定や、待機・スロットの変更は行いません。
再試行期限の観測は、`retry_not_before` を持つ inbox 行を、Issue の重複と状態も含めて保持し、
スキャン時点の残秒を示します。タイムゾーンの差を反映し、端数秒は切り捨て、期限経過は 0 秒です。
時刻の欠落・不正や `unknown` は残秒を不明とします。開始時刻がないため保留の継続時間も不明です。
Monitor が取得できなければ null、該当行がなければ空リストを返します。取得元を保持し、不正な
時刻文字列と理由の自由文を省きます。この観測で保留が有効であることや起動・解除の許可を証明せず、
再試行ゲート・待機・スロット・優先順も変更しません。
マイルストーン診断では、明示的な所属 Issue と、一意かつ空でないマイルストーン ID を
参照するロードマップ項目の Issue について、Monitor inbox の観測状態と未観測番号も示します。
これは参照の事実であり、目標との整合を証明するものではありません。保留・失敗・claim 競合を
状態から確認できますが、観測の欠落や Merged / Released を健康・完了の証拠にはしません。
目標診断でも、一意かつ空でない目標 ID と完全一致する明示的なリンクについて、
ロードマップ参照・Issue 番号・inbox の観測状態・未観測 Issue を示します。
不正なリンクも参照の事実として残し、マイルストーンや区分だけのリンクは所属を広げません。
空または重複した目標 ID ではリンクを解決しません。観測から目標達成や指標の裏付けを
証明せず、正本となる情報源が接続されるまで `metric_evidence` は不足として残します。
各 Goal の `missing_inputs` は、空欄または空白文字のみの outcome、metric、target、
horizon、owner、status も示します。ロードマップへ未リンクの Goal も対象です。
不足項目を埋めるよう PM に依頼できます。情報が揃っても outcome の達成証明にはなりません。
目標・マイルストーン・ロードマップの変更では、理由・revision・変更前後の値・直接影響する
Issue も計画の監査記録に残します。目標変更では、ロードマップ項目からその目標へ明示的に
紐付く Issue のみが対象です。この記録だけでは、依存先や担当エージェントへの
通知済みとは判断しません。
リスク・意思決定・ポリシーの編集も、credential を除去した変更前後の値を同じ監査記録に
残します。これらの記録は直接の Issue 紐付けを持たず、その不足を明記します。影響 Issue が
空でも「影響なし」を意味しません。定期作業の編集も変更前後の設定を残し、設定の冪等キーは
監査記録へそのまま複製せず digest にします。定期作業も直接の Issue 紐付けを持ちません。
この記録だけでは作業をスケジュールせず、重複防止や通知済みの証明にもなりません。

PM 自身は実装エージェントを起動しません。対象 Issue をキュー先頭へ移動して
スキャンを要求するだけで、実際の起動は Issue Monitor の既存 claim/slot 経路が
担うため、多重起動の防止機構はそのまま維持されます。

- プロジェクトを開くと自動起動します。プロジェクト単位で opt-out できます。
- PM 設定の **Pause / Resume** で自律ループを一時停止・再開できます。停止状態は
  再起動後も維持され、PM との会話、Issue Monitor、稼働中の agent は継続します。
  Resume は最新の状態を再確認します。JSON operations `pm.pause` / `pm.resume`
  でも操作でき、`pm.status` は登録状態と別に `paused` を返します。
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
- 稼働中の登録済み PM は `workspace.close` で指定した非稼働 Work を閉じられます。
  `work_id` と `close_kind: "done"` または `"discarded"` を指定し、GUI と同じく
  手動 close が許可され、その Work の稼働 agent がいないことを確認します。
  終了履歴を記録し、branch と worktree は保持します。

PM に Board Discussion 共有の有効化や単独 Discussion の作成、`develop` / `main` の必須チェック変更、
全プロジェクト共通の Auto-apply updates 変更を依頼できます。操作対象を明確にした
指示の記録、または OS ネイティブの承認ダイアログで許可します。**Approve once** で
今回だけ許可するか、同じリポジトリ・操作種別の許可を記憶できます。保存した許可の確認・
取り消しは PM に `pm.approval.status` / `pm.approval.revoke` で依頼できます。
必須チェックの変更には `repository.required_checks.update` を使い、ブランチの strict
設定と、残すチェックの app との関連付けを維持します。

同じ範囲付き許可で、`pr.reopen` による指定 PR の再開、`pr.review.submit` による
GitHub review、`release.metadata.set` による既存 release の名前・説明・prerelease の変更、
`repository.settings.set` による対応済みの可逆な metadata・機能・merge 設定・archive 状態の
変更も依頼できます。topics の置換は `topics` だけを指定します。既存の review・CI チェックは
維持し、release の公開・asset・latest の選択には既存の gated workflow を使います。

リポジトリの自動マージ対象から外れる外部 fork の PR は、レビュー済みの PR 番号と
head SHA をオーナーが同じ範囲付き許可で PM に委任できます。`pr.auto_merge.enable` に `number` と
`reviewed_head_sha` を指定すると、現在の PR・CI・review 状態を確認し、その SHA に限って
GitHub の auto-merge を有効にします。監査記録には PR 番号・SHA・許可の根拠を残します。
head が変わった場合は再レビューと新しい許可が必要です。この経路が利用できない、または
委任が無い場合は人によるマージが必要です。`pr.list` はマージ提案を繰り返す代わりに、
外部 fork のため自動化の対象外になり得る理由と代替経路を示します。

明示的に許可した PR の close、ビルドキャッシュ削除、provider 承認への応答には、PM が
`pm.operation.execute` を使います。対象は既存の `pr.close` / `worktree.gc_build_artifacts` /
`pm.message.send` の3種類に限られ、それぞれの既存のチェックを維持します。

同じマシンの別プロジェクトの PM と相談したい場合は、相手プロジェクトの絶対パスを
添えて自分の PM に依頼してください。登録済み PM は `pm.peer.send` で依頼・応答・
通知を直接送り、未読メッセージは相手の次の PM prompt に届きます。双方の
**Reports** でやり取りを追え、相手が不在の場合や期限を過ぎた応答待ちも表示されます。
各 PM は自分のプロジェクト内で判断・操作し、メッセージによって他プロジェクトの
エージェントを操作する権限は与えられません。

設計は SPEC [#3431](https://github.com/akiojin/gwt/issues/3431) にあります。

## Knowledge、Search、Managed Skills

gwt は project knowledge を Agent workspace の近くに置きます。

- JSON operation `issue.spec.read` は GitHub Issue-backed SPEC をローカル cache から読みます。
- JSON operations `issue.view` と `issue.comments` は gwt CLI surface から
  cache-backed Issue access を提供します。
- `gwt-search` は共有 ChromaDB runtime を通じて SPEC、Issue、source files、
  docs、PM Reports を検索します。index が無い場合は必要に応じて build され、desktop app は
  管理対象 Python search runtime を修復できます。
  Issue index の再構築中も直前に公開された世代を検索できます。複数 scope の検索は
  利用可能な結果を返し、除外した scope と理由を通知します。JSON 応答には
  `partial` と `unavailable_scopes` が含まれます。
  `reports` scope では保存済みの digest と Q&A の質問・回答・Issue 参照を検索できます。
  Index window は report の kind・日時・関連 Issue を表示し、**Open** で PM の
  Reports view へ戻れます。
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
- `gwt-feedback` — 任意のプロジェクトから gwt の問題を報告する

gwt の不具合や workflow の問題は、PM と実装 agent のどちらも
`gwt-feedback` で対象 operation、拒否文の原文、再現手順、回避策を集めて
報告できます。JSON operation `feedback.create` の宛先は常に
`akiojin/gwt` で、診断コンテキストをローカルで自動収集します。
ローカル receipt に保存します。実際の gwt リポジトリ以外では受付後に
即座に返るため、本来のプロジェクト作業を続けられます。送信には native
dialog でユーザーが **Send feedback** を選ぶ必要があります。取消や desktop
不在でも内容はローカルに残ります。gwt 自体の開発では承認を求めず同期送信し、
Issue 番号と URL を返します。

ローカル receipt は完全な原文を保持します。公開内容では既知の認証情報、
環境変数の代入値、プロジェクト識別情報、URL を含むパス、ソースコードを自動で伏せます。
公開する環境情報は gwt/gwtd 版数と OS・アーキテクチャのみとし、報告元のメタデータと
任意の追加本文は含めません。スクロールできる承認画面は両方の選択肢を常に表示し、
除去項目とマスク率、過半数が伏せ字の場合の警告を示します。
preview と送信に同じ内容を使い、伏せ字以外の拒否文は切り詰めません。
判定は保守的ですが任意の秘密をすべて検出する保証はなく、原文 receipt は公開しないでください。

返された request id を `feedback.status` に渡すと、送信先 Issue の番号と
URL を確認できます。類似する既存 Issue があればコメントを追記します。
`feedback.retry` は declined / failed の receipt だけ再試行し、外部プロジェクト
では改めて承認を求めます。送信結果が uncertain の場合は内容を保全し、
自動再送しません。

報告は PM が分類するための証拠として扱います。確認できた不具合は、要件を整理した
別の修正 Issue に紐付け、原文の報告から直接 agent を起動しません。機能追加や
判断が曖昧な報告はリポジトリ owner の採用判断を待ち、判断と修正先を報告に記録します。

SPEC と通常 Issue は同じ検索索引を使い、SPEC は見出しチャンクとして一度だけ格納します。
`gwtd` JSON `search` の `scopes:["specs"]` は SPEC、`scopes:["issues"]` は
通常 Issue、`scopes:["specs","issues"]` は両方を検索します。既定では両種と他の検索
スコープを含みます。`gwt-spec-search` / `gwt-issue-search` も同じフィルターを使います。
どちらかのスコープを再構築すると、ローカル Issue キャッシュから旧索引を移行します。
旧 SPEC 専用ストアは参照されなくなるため、手動で削除する必要はありません。

Managed hooks は user hook を保持しながら、Agent state、workflow guardrails、
Board reminders、discussion/plan/build Stop checks、coordination-event summaries
を追加します。

agent の停止や pane close が Board の未読を報告した場合は、同じ agent session で
JSON operation `board.show` に `params:{"all":true}` を渡して本文を読み、再試行します。
自動 reminder の注入は読了として扱われません。実装 agent の判断依頼は `agent:pm`
宛に送ります。agent の Board 投稿から `user:<id>` を指定できるのは登録済み PM だけです。

Board の decision が idle の agent session（`session:<id>`）を mention すると、
gwt は裁定を読んで続行するよう対象 agent に 1 行の通知を送ります。
Board が再更新されても同じ通知は繰り返しません。

JSON operation `hook.health` では、gwt の GitHub mutation policy と矛盾する外部 skill の
コマンド、該当ファイル、代替操作を確認できます。既定では有効な Claude plugin を検査します。
検査する skill/command directory を明示的に選ぶ場合は、`.gwt/external-skill-roots.json` に
directory path の JSON 配列を保存します。この設定は plugin の自動検出を置き換え、
相対 path は worktree 基準です。検査したい root をすべて含めてください。ファイルを削除すると
自動検出へ戻ります。検査が部分的、または実行できない場合は明示されます。仕様は
[SPEC #3393](https://github.com/akiojin/gwt/issues/3393) を参照してください。

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

`errors.list` は既定で現在のプロジェクトのエラーだけを返し、同じプロジェクトの
別 worktree の記録も含みます。`project_root` で記録済みの root を明示して
絞り込めます。起動時の共通設定などマシン全体のエラーは
`scope: "host"`、帰属不明の記録は `scope: "unknown"`、全スコープは
`scope: "all"` を明示して取得します。`project_root` を指定した場合は
host・unknown 行を含めず、所属プロジェクトを推測で補いません。
破損した台帳行は、ファイル名と行番号を持つ `ledger_corruption` の host 診断として
返します。timestamp は破損を検出した時刻で、元の行は変更しません。

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

Hub またはシステムトレイの `Clone from GitHub…` から clone を開始できます。
clone modal では GitHub HTTPS / SSH URL を直接入力するか、`gh search repos`
による repository 検索から候補を選択し、保存先の親フォルダを指定します。新しい
プロジェクトは `<parent>/<project>/` に作成され、`<project>.git/` bare リポジトリ
と initial worktree が配置されます。remote に `develop` が存在する場合は
`develop` worktree を作成し、存在しない場合は remote default branch を使います。

Clone を実行するとブラウザタブを確保し、成功後にそのタブでプロジェクトの URL を
開きます。Hub はそのまま残ります。ポップアップがブロックされた場合や確保した
タブを閉じた場合は、Hub の完了リンクから開けます。失敗時は確保したタブを閉じて
理由を表示します。同じリポジトリが Open または Recent に登録済みなら clone を
拒否し、**Open existing project** から既存の URL を開けます。

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

### 旧形式の Work event 衝突を復旧する

未追跡の `.gwt/work/events/` shard が上書きされるとして Git merge が拒否された
場合は、対象ブランチを fetch してから、プロジェクトディレクトリで次の JSON を
`gwtd` の標準入力へ送ります。

```json
{"schema_version":1,"operation":"workspace.events_reconcile","params":{"reference":"origin/develop"}}
```

既定の dry run は、復旧可能な旧形式の successor tombstone 衝突を報告します。
`"dry_run":false` を加えて再実行すると、双方の原文を報告された home 内の archive に
保存し、内容ごとの ID で双方の event を公開します。その後 Git merge を再実行します。
追跡済み shard、通常の event、異なるブランチの tombstone は拒否し、既存のソースや
archive を上書きしません。

既存 Work に絶対 worktree path と `.` の container が重複している場合は、
`workspace.work_repair` の `params.work_id` で対象を指定して修復内容を確認できます。
既定は dry-run で、`"dry_run":false` を加えると適用します。記録済み Git worktree と
永続 event の証拠が一致する場合だけ修復し、terminal 状態とソースファイルを保持します。
証拠の欠損や曖昧性がある場合は拒否します。

## GitHub Discussions でローカル Board の節目を共有する

別マシンと `decision` / `blocked` / `handoff` を共有するには、PM に scoped な
`board.config.set` で共有を有効化するよう依頼します。`category:"General"` または
別のカテゴリー名を指定でき、Discussion 番号や手動での Discussion 作成は不要です。
現在の Board provider が Local 以外なら `provider:"local"` を明示して切り替えます。
既存の scoped authority を必要とし、全体設定や他プロジェクトの設定は維持します。
手動で opt-in する場合は `.gwt/work/board.toml` を設定します。

```toml
provider = "local"

[github_discussion]
owner = "your-org"
repo = "your-repo"
category = "General" # 省略時の既定値
```

各マシンで同じリポジトリとカテゴリーを指定し、GitHub Discussions を有効にして、
読み書き権限を持つ `gh` 認証を用意してください。共有対象の Board root ごとに
Discussion を1つ作り、title は owner Issue 番号と短い要約、本文には Board entry ID と
owner Issue へのリンクを置きます。共有対象の返信はその Discussion のコメントになります。
`status` / `claim` は返信もローカルに留まり、共有種別は設定で変更できません。
永続履歴にも root が見つからない返信や、root が共有対象でない返信はローカルに留まります。
中間の親が local-only や表示範囲外でも、共有対象の子孫は配送します。
旧設定の `number` は読み込み互換として残りますが、単一の配送先を指定する用途には
使わず、新しい投稿は話題ごとの Discussion へ配送します。既存の一括 Discussion は残します。
作成は投稿元のマシンに限定し、新しい投稿の出自記録、または旧 root の Session 履歴で
判定します。出自を証明できない旧 root は診断を残してローカルに留まります。opt-in 後の
新しいローカル root は出自を自動記録します。複数マシンの同時移行での重複作成を避けるためです。

Board の読み書き時に最大1分に1回バックグラウンド同期を起動します。明示的な同期と
診断の確認は `{"schema_version":1,"operation":"board.sync","params":{}}` を
`gwtd` の標準入力へ送ります。受信した投稿は既存 Board のフィードに表示されます。
GitHub が返す投稿者の所属が `OWNER` / `MEMBER` / `COLLABORATOR` の root / コメント
だけを取り込み、それ以外や不明な所属は取り込みません。通常のコメントは安定した
remote ID を持つ Board 返信になります。encoded 投稿は escalation を開閉できるため、
所有者・組織メンバー・collaborator を調整相手として信頼できるリポジトリを指定してください。
公開リポジトリでは共有する Board 本文も公開されます。

同期は bounded best-effort です。追跡は最大50話題で、超過した話題はローカルに留め、
`board.sync` に上限を表示します。各 tick で repository を1ページ、話題を最大5つずつ
順に読み、作成・投稿は合計最大5件に制限します。repository の発見が完了するまで
作成せず、作成応答を失ったときも entry ID を照合してから再試行します。GraphQL の
実際の cost は既存 GitHub budget に記録し、予算不足なら同期を延期します。
送信候補はローカルの現在の表示範囲（最大500件）です。長期障害中に範囲外へ出た
未送信投稿は遡って配送しません。レート制限・認証失効・ネットワーク障害でも
ローカル Board は利用でき、診断を残して5分後に共有 budget を尊重して再試行します。
停止中の相手は後の同期で保存済み投稿を受信します。共有を停止するには
`[github_discussion]` を削除してください。

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

PM ウィンドウの **Reports** は、明示的に投稿された Markdown の報告を種別・時刻・
**Copy** ボタン付きで表示します。Q&A 報告では質問・指示と回答・行動結果を分けて表示し、
**Report kind** で種類を絞り込めます。閲覧専用で、ターミナル操作には **Execution log** を
使います。報告は `~/.gwt/projects/<project-hash>/project-state/pm-reports.jsonl` に保存します。
指示を記録した Q&A は同じ machine-local project store の `project-state/user-instructions.json`
を正本とし、1 回の原子的な保存で報告と指示を保持します。再起動後も保持し、自動失効・件数制限は
ありません。空でない報告本文は 1 件 64 KiB までです。

登録済み PM は `pm.report.post` の `kind`（`progress` / `decision` / `blocker` / `qa`）と
`body` で投稿します。`qa` には、ユーザーの質問・指示を要約した空でない `question` が必須で、
`body` に回答・取った行動・結果を記載します。`pm.report.list` で履歴を読み出せます。標準の gwtd JSON envelope を
使用します。投稿成功は永続化の確認を意味し、失敗時は明示的なエラーを返します。

PM の Q&A 投稿には、原文 `original`、種別 `kind`（`standing` / `one_off` / `question`）、
`actions`、`issue_numbers` を持つ `instruction` を含めます。任意の `supersedes` に旧指示の ID を
指定すると、旧レコードを残したまま `superseded_by` で置換先を記録します。秘密値は保存前に伏せます。
Memory 検索で原文と結果を検索でき、取り消されていない常設指示は PM の起動時と各ターンに再読込します。

**PM status** カードと報告履歴は同じ領域でスクロールします。カードは要約表示で始まり、
展開すると **PM notes**（PM の説明）と **Live facts**（稼働窓・キュー先頭・人への引き継ぎ件数・
リリース PR・未適用の更新）を表示します。展開状態は窓ごとに保持し、取得できない情報は
**Unknown** と表示します。UI の固定文言は英語、投稿本文は書かれた言語のまま表示します。
PM は `pm.status.set` で説明を更新します。`items` は 1〜3 件で、
`kind`（`working` / `waiting`）、`text`、RFC3339 の `started_at` を指定します。
`next_trigger` は必須、`user_action` は任意です。更新停止の警告は既定で 15 分後に表示し、
`stale_after_minutes` で変更できます。`pm.status` は最新の説明・保持された状態履歴・
報告の診断情報を返します。説明と Monitor の状態が同じ場合は時刻を更新しません。
人間との PM の各ターンは、Q&A 報告の保存に成功してから終了します。定期的な自動サイクルは
この終了条件の対象外です。

常駐 PM は gwt 所有の runtime ディレクトリで起動します。リポジトリの skill、
hook、`AGENTS.md`、`CLAUDE.md` は project の data として読めますが、PM の設定には
読み込まれません。実装 agent は従来どおり project 設定を使います。既存の PM 会話は
自動移行せず、次回の PM セッション起動時から分離されたディレクトリを使います。

PM には coordination、agent 観測、検索、要望受付、計画の skill と、それらが使う
検索・登録 helper を配布します。実装・検証・PR ワークフローの skill は実装 agent が
使用します。managed asset の再生成時に、PM runtime に残った実装用 skill と
`gwt-*` command alias を除去します。実装 agent の skill 一式は従来どおりです。

project 固有の規約を明示的に渡すには、
`~/.gwt/projects/<project-hash>/project-state/pm.json` の他のフィールドを保持したまま、
`settings.project_policy_files` を設定してください。
例: `"project_policy_files": ["docs/pm-policy.md"]`。パスは PM の project checkout からの
相対パスで、既定は空リストです。選択した内容は managed asset の再生成時に既存の
`gwt-pm` skill へコピーされ、runtime に project への symlink は作りません。
指定を外した内容は次回の再生成で除去されます。

## サーフェス操作

- レールから **Issues**、**Agents**、**Board**、**Settings** を選ぶと、コンテンツ領域全体に表示します。
- **PM** は独立したレール入口から開き、単一ペインで表示します。
- 各面に窓のクローズ・最小化・移動・リサイズ操作はありません。Agent タブを閉じても表示だけが閉じられ、セッションは動作を続けます。
- プロジェクトバーの **Split view** で左右のペインを表示し、それぞれのサーフェスを選びます。
- **Close split** でアクティブなサーフェスを単一ペインに戻します。
- **Agents** では同じライブターミナルを維持して、**All agents** と個別タブを切り替えます。

## Agent の巡回

`Cmd/Ctrl+Shift+Right` と `Cmd/Ctrl+Shift+Left` で、個別 Agent タブを
状態順（running/starting → waiting/idle → その他）に選択します。
Issue 内に表示された Agent と Shell のターミナルも含み、PM はスキップします。
分割表示を維持し、選んだターミナルに入力フォーカスを移します。
選択中のタブを閉じると Agent を停止せず All agents に戻り、巡回で同じターミナルを
再び開けます。

Agent のカードとタブのタイトルは1行表示で、先頭の名前は固定されます。
作業内容を横にスクロールするか、タイトルへのホバー・キーボードフォーカスで全文を読めます。
Issue と Project Manager の設定項目も、ホバーとキーボードフォーカスで説明を表示します。

## Operator デザイン言語 (SPEC-2356)

Operator Design System 採用後、gwt は editorial-industrial 系タイポグラフィ
(本文 `Mona Sans` / ディスプレイ `Hubot Sans Condensed` / 等幅 `JetBrains Mono`)
を中核とした単一の mission-control サーフェスとして再設計されました。
既定の type scale は開発時の可読性を優先し、terminal text、ID、path、
counter、密度の高い作業サーフェスを長時間でも読み取りやすくします。
一方で display typography は見出しと chrome label に限定して Operator らしさを
保ちます。Project Bar / Command Rail / Status Strip / Command Palette /
Hotkey Overlay / Drawer モーダル / サーフェスパネル の全クロームが
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
| `⌘G` | Issues サーフェスを focus |
| `⌘L` | Logs サーフェスを focus |
| `⌘?` | Hotkey Overlay (cheat sheet) を toggle |
| `Esc` | 開いている Palette / Overlay / Drawer / Dropdown を閉じる |

レールには **PM** の独立した入口と、ラベル付きの **Issues**、**Agents**、
**Board**、**Settings** が並びます。選択したサーフェスは固定ペインに表示し、
**Split view** で必要に応じて2つ目のペインを追加できます。Status Strip は
コンテンツ領域の下に表示します。その他の操作は Command Palette またはホットキーから使えます。

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

GUI と daemon は、Session 履歴と復旧情報を保持しながら、放棄された runtime namespace を
自動的に整理します。回収のタイミング・保持方針・診断については
[Issue #5384](https://github.com/akiojin/gwt/issues/5384) を参照してください。

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

ローカル Git 履歴なしで、直近 50 件の completed workflow run（失敗・取消も含む）の
queue 待ちと依存スケジューリングを分けて計測できます。

```bash
python scripts/ci_throughput.py --repo akiojin/gwt --queue --save target/ci-queue.json
python scripts/ci_throughput.py --input target/ci-queue.json
```

`--before` は run 作成時刻の上限（その時刻も含む）を固定し、`--workflow test.yml` は
対象 workflow を絞ります。run 開始待ち、job 開始待ち、run 開始から job 作成までの時間（依存・
スケジューリング）を分け、各 run の最長 job も示します。job 開始待ちには concurrency
group 待ちも含まれ、API の時刻だけでは runner 容量不足と断定できません。skip・取消 job と
再実行で再利用された時刻は job 待ち時間の分布から除外します。再実行の run 開始待ちは、
作成時刻が元の attempt に属するため取得不能として扱います。

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
通知は非フォーカスの窓でも動き、focus を更新条件にしません。`subscribe` は
terminal packet を含む各更新を同期で消費します。描画用の `subscribeFrame` は初回の
選択値を即時描画し、以後は変更された selector をストアごとに 1 回の callback へ
まとめ、最新 snapshot を描画します。animation frame が停止しても 100 ms timer で
進み、購読解除や dispose 時は予約を取り消します。Usage のステータス表示・popover・
設定・Workspace 表示がこの経路を使用します。
共通の `ui-content.js` は content の型から plaintext または backend で sanitize 済みの
Markdown 描画を選びます。モデルへの取り込み時に `{ type: 'markdown', body, body_html }`
をレコードの `content` に設定します。各面は確定した content を `renderUiContent` に渡し、
Markdown の選択や描画関数の注入を行いません。移行対象と受け入れ条件は
[SPEC-5016](https://github.com/akiojin/gwt/issues/5016) を参照してください。

### 重量級検証の容量制御

ホスト全体の verification lease を取得するのは canonical `verify.run`
だけです。`verify.plan` で検証行列を登録し、`verify.run` で実行します。
各 Heavy コマンドの実行時に取得・解放し、Light コマンドは他の run と並行できます。
未実行の Light を先に進め、Heavy の相対順序を保ち、gwt の成果物復旧は最後に実行します。
残りの最初のコマンドの取得待機が時間切れになると、記録を置き換えず既存の記録を保持します。
途中の時間切れでは、先行コマンドの結果を未完了・非 PASS の `deferred` 記録に残します。

単一 package の library は、正のテスト名 filter があれば Light です。
Nextest の `-E 'test(=module::case)'` または
`--filterset='test(=module::case)'` も、厳密一致 filter 1 個なら対応します。
名前に使えるのは ASCII 英数字・underscore・colon です。複数の filterset、広域・複合式、
package / target の選択範囲を広げる指定は Heavy のままです。

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

非 resident 予約は最終 refresh から3分間、FIFO の優先権を保持します。
その後は verification lane の後続が、exclusive admission を要するコマンドも含めて
入場できます。status は bypass 理由を表示し、元の到着順位と期限は保存します。
同じ完全な matrix で `verify.run` を再提出すると、元の優先権を回復します。
新しい attempt は Cargo artifact 待ちに入る前に既存の deferred 予約を引き継ぎ、
古い attempt の cancel は後継の予約を削除しません。

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

検証の子プロセスでは既定で Git の端末プロンプトと Git Credential Manager の対話を無効にします。
GUI や端末での認証入力が必要になった場合、入力を待たずにエラーを返します。
キャッシュ済みの認証情報は引き続き使用できます。

同一プロジェクトの holder が回収候補として報告された場合は、`verify.lease.release` に
`params.lease_id` と空でない `params.reason` を渡します。作業が残っていないことを
再確認してから回収します。実行中の workload は保護されるため、自分の試行は上記の取消を使います。

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

### 性能検証の実行環境

canonical な headed E2E コマンドは計測環境も記録します。ブラウザ以外の性能検証は、
`verify.plan` と `verify.run` のコマンド項目にコマンド単位の環境変数
`GWT_VERIFY_PERFORMANCE=1` を付けます。別途起動した GUI を測る場合は、
`GWT_VERIFY_TARGET_PIDS=1234` も指定します（複数の PID はカンマ区切り）。

Windows では、検証を含む gwt pane Job の CPU hard cap が解除されたことを確認してから
計測します。canonical の全 slot と旧 verification lane が対象で、holder がいなくなると
設定された上限を復元します。プロセス優先度と pane 終了時の回収は維持します。
古い GUI の Job 祖先を検査できない場合は、更新後の runtime で gwt を再起動してください。

コマンドの `measurement` 証跡には、対象 PID と開始時刻、観測した Job cap、
サンプリングした CPU 時間・使用率、admission 待ち時間、cap 解除待ち時間を記録します。
待ち時間は admission・事前確認の所要時間であり、OS スレッドのスケジューリング待ち時間の
推定値ではありません。runner と指定した GUI は `required_uncapped: true` とし、
それ以外の子プロセスは CPU の参考情報として通常の資源制御を維持します。
解除が必要な対象に cap が残る場合や、その環境を検査できない場合は
`ENVIRONMENT_INCOMPATIBLE` となり、完了や Ready PR の根拠にはできません。
診断された環境を修正して、登録済みの同じ検証行列を再実行してください。

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

source 内訳には、継続ページも含めて `issue-cache.refresh` や
`issue-monitor.queued-issues` などの呼び出し元名が表示されます。Monitor の
キャッシュ更新に失敗した場合、一般一覧の取得済みページを既存の15分の期限内で再利用します。
キュー一覧と明示的な更新は、引き続きリモートから取得します。

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
