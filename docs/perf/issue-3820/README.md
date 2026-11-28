# Issue #3820 T-006: 未測定経路の暫定値

SPEC #3820 の inventory section のうち、常時計測で値が得られない経路を手動で計測した記録。
値はすべて「暫定」であり、経路の改善完了やバジェット達成を主張するものではない。
経路の定義とバジェットの所在は inventory section と [`../README.md`](../README.md) を参照する。

## P02 プロジェクト open

### 結論

`route:project.open` の production 計装は回帰していない。隔離インスタンスで 10 回の open がすべて
`route:project.open` を記録した。共有 perf log で 2026-11-10 以降サンプルが無いのは、
このホストで新しい open 操作が無かったためと判断する。

| n | p50 ms | p95 ms | 最悪 ms | バジェット ms | 判定（暫定） |
|---:|---:|---:|---:|---:|---|
| 10 | 62.259 | 75.206 | 75.206 | 2000（`DEFAULT_PROJECT_OPEN_BUDGET_MS`） | 予算内 |

p50 / p95 は nearest-rank。n=10 のため p95 は最悪値と同じ。

### 条件

- 対象: checkout `6a15eebcb697bb20364fe29819b8a83ce245df04` の `target/debug/gwt`（debug build、gwtd 9.178.0）。
- host: macOS 27.2.1、Apple M5 Max（`p02-environment.txt`）。2026-11-27T17:21Z。
- 起動: `--no-tray --no-open`、fresh HOME。`~/.gwt/runtime`、`.gitconfig`、`.config` だけを symlink した。
- fixture: 1 commit・1 ファイルの git repo を 10 個用意し、それぞれ一度だけ開いた。GitHub remote は無い。
  各 repo の PM auto start と Issues の自動実行は browser-check の seed で無効にした。
- 操作: Hub WebSocket から production の frontend event `reopen_recent_project` を 1 件ずつ送り、
  isolated HOME の perf log に新しい `route:project.open` が書かれるのを待ってから、1 秒置いて次を送った（`p02-drive.mjs`）。
- 計測の起点と終点: route の定義どおり、`request_project_open` の受付から、同じ request の
  `commit_prepared_project_open` 完了まで。送信から perf log で観測するまでの時間（101〜104 ms）は
  driver の 100 ms polling 間隔を含むため、経路の値には使わない。

### 限界

- fixture は最小の repo であり、大きな repo、多数の worktree、GitHub remote がある場合の値ではない。
  経路の現在値としては下限側に偏る。
- 画面上の反映（新しい tab の描画）は終点に含まない。
- browser-check の hook repair / audit は実行していない。このホストでは Codex hook trust の登録に
  `~/.codex/config.toml` への書き込みが必要で、自律実行ではリポジトリ外への副作用になるため。
  この計測は agent を起動しないので、hook の状態は計測値に影響しない。
- 起動ログの error 行は 1 件で、installed gwt が OAuth callback port 8765 を使用中という通知だった。

### ファイル

- `p02-drive.mjs`: open を順に送って perf log を待つ driver。
- `p02-driver.json`: driver の出力（各 open の route 値、timeout の有無、Hub の error message）。
- `p02-project-open-samples.jsonl`: isolated perf log から抜き出した `route:project.open` の生サンプル。
- `p02-environment.txt`: checkout SHA、version、OS、CPU、採取時刻。

## P07 / P17 / P18 terminal 系

### 結論

Issue #8043 の解消（PR #8056）後、隔離インスタンスの実 Shell PTY で 3 経路を計測した。headed Chromium の
dark / light で各 30 回、計 60 回。3 経路とも参照バジェット内（暫定）。console / page error と
Shell への入力配送エラー（`window_command_error`）は両テーマとも 0 件。

| 行 | 経路 | n | p50 ms | p95 ms | 最悪 ms | 参照バジェット | 判定（暫定） |
|---|---|---:|---:|---:|---:|---|---|
| P07 | prompt 後の操作回復 | 60 | 29.7 | 37.0 | 38.1 | #3777 の 250ms 基準 | 予算内 |
| P17 | terminal 描画 | 60 | 17.8 | 24.9 | 25.9 | #3700 frame 16ms は frame 部分のみ。end-to-end 予算は未定義 | 判定不能（予算未定義） |
| P18 | terminal scroll | 60 | 20.4 | 23.1 | 25.6 | #3700 ui_response 100ms ＋ frame 16ms | 予算内 |

テーマ別（n=30 ずつ）:

| 行 | dark p50 / p95 / 最悪 ms | light p50 / p95 / 最悪 ms |
|---|---|---|
| P07 | 29.8 / 34.0 / 34.8 | 29.5 / 37.5 / 38.1 |
| P17 | 18.0 / 22.5 / 24.7 | 17.8 / 25.4 / 25.9 |
| P18 | 20.7 / 23.2 / 25.6 | 20.3 / 22.9 / 24.6 |

p50 / p95 は nearest-rank。

### 条件

- 対象: checkout `2def6e8d121a0db95525b0dffb7bab11e05486e4` の `target/debug/gwt`（debug build、gwtd 9.179.0）。
- host: macOS 27.2.1、Apple M5 Max。2026-11-28T00:44Z〜00:46Z（`terminal-environment.txt`）。
- 起動: `--no-tray --no-open`、fresh HOME。`~/.gwt/runtime`、`.gitconfig`、`.config` だけを symlink した。
  PM auto start と Issues の自動実行は seed で無効にした。Hub WebSocket の session keeper を併用した。
- fixture: 空 commit 1 件の repo から作った linked worktree を Hub で開き、Project owner が Healthy に
  なってから Shell window を 1 つ作った。owner の attach grace（15 秒）が過ぎるまで 16 秒待ってから計測した。
- browser: Playwright の headed Chromium、viewport 1440×900。macOS で隠れた window の animation frame が
  止まらないよう `--disable-backgrounding-occluded-windows` などを付けた。
- driver: `crates/gwt/playwright/tests/terminal-latency-inventory-live.spec.ts`。環境変数
  `GWT_PLAYWRIGHT_BASE_URL` / `GWT_PLAYWRIGHT_CHECK_HOME` / `GWT_PERF_INVENTORY_OUT` が無い場合は skip する。
  CI の回帰 gate ではない。
- 起点・終点（時刻はすべて page の `performance.now()` と同じ時計）:
  - P07: Enter の keydown の `event.timeStamp` から、続けて打った `Q` が送信したコマンドの次の行に
    表示された最初の animation frame まで。コマンドは毎回異なる no-op（`: P07_<n>`）。
  - P17: マーカーを含む出力が page の WebSocket に届いた `message` event の `timeStamp` から、
    マーカーが xterm buffer に入った後の最初の animation frame まで。マーカーは shell が展開して初めて
    現れる（`echo P17M<n>X$((40+2))`）ので、入力の echo では成立しない。出力は stream chunk
    （`terminal_output`）と画面全体の `terminal_snapshot` の両方で届いた。60 回中 45 回が
    `terminal_output`、15 回が `terminal_snapshot`。
  - P18: `seq 1 3000` で scrollback を作り、`wheel` event の `timeStamp` から xterm の `viewportY` が
    変わった最初の animation frame まで。上下 400px を交互に送った。

### 限界

- 最小の fixture と、ほかに負荷のないインスタンスでの値。agent の出力が多いときや、多数の pane が
  動いているときの値ではない。経路の現在値としては下限側に偏る。
- P07 は Playwright が Enter と `Q` を順に送る時間（CDP の往復）を含むため、上限側に偏る。
- P17 の起点は browser への到着であり、PTY の書き込みから WebSocket 送信までの backend 区間は含まない。
- 「最初の animation frame」は、その frame の callback が条件を観測した時刻であり、実際に画面へ
  合成された時刻との差（1 frame 未満）は測っていない。
- browser-check の hook repair / audit は P02 と同じ理由で実行していない。計測は agent を起動しない。
- driver の初版は Shell の前面化を待たずに reload しており、Settings が前面のまま Shell の prompt が
  見えずに失敗することがあった。前面化を待つよう直した後の 1 回の実行で両テーマが通り、その値を採用した。

### ファイル

- `terminal-dark.json` / `terminal-light.json`: テーマごとの生サンプル（`samples_ms`）、分位、
  P17 の到着 kind の内訳、pane 宛て message の種類別件数、console error と配送 error。
- `terminal-environment.txt`: checkout SHA、version、OS、CPU、採取時刻。
- `check-terminal-evidence.mjs`: 生サンプルから分位を再計算し、上の 2 つの表と一致することを確かめる（`node docs/perf/issue-3820/check-terminal-evidence.mjs`）。

## P20 workspace merge 状態の反映

### 結論

PM の指示（2026-11-28）で計装を追加した。merge 状態の scan は Work events ingest の event-loop continuation
（`handle_work_events_ingested`）からだけ起動されるため、既存 route `work_events.ingest` の phase
`phase:work_events.ingest.merge_status` として記録する。#3700 にこの経路のバジェットが無いため、
route にせず予算なしの phase とした（バジェット値を独自に決めない、[`../README.md`](../README.md) §1）。

| n | p50 ms | p95 ms | 最悪 ms | バジェット | 判定（暫定） |
|---:|---:|---:|---:|---|---|
| 10 | 49.116 | 75.932 | 75.932 | 未定義（#3700 へ要求） | 判定不能 |

p50 / p95 は nearest-rank。n=10 のため p95 は最悪値と同じ。同じ 10 回の `route:work_events.ingest` は
p50 9.013 / p95 14.798 ms。

### 計測の起点と終点

- 起点: merge worker の 1 pass の開始（scan cache の lock 取得前）。合流した要求は次の pass で処理されるため、
  pass が要求の単位になる。
- 終点: `WorkMergeStatus` を GUI の event loop へ送出した直後。
- 含まないもの: 送出から GUI が `apply_work_merge_status` を実行するまでの待ちと、その handler 自身。
  handler は既存の GUI dispatch 計測（50 ms 超の handler だけ記録）が扱う。projection の再配信と画面の描画も含まない。
- 同じ pass の後続処理（tip subject、PR title の取得）は含まない。

### 条件

- 対象: checkout `f9012ab5fd` に本 PR の計装を加えた `target/debug/gwt`（debug build、gwtd 9.179.0）。
- host: macOS 27.2.1、Apple M5 Max。2026-11-28T01:57Z〜01:58Z（`p20-environment.txt`）。
- 起動: `--no-tray --no-open`、fresh HOME。`~/.gwt/runtime`、`.gitconfig`、`.config` だけを symlink した。
- fixture: 1 commit の repo を 10 個、各 repo に linked worktree を 3 個。GitHub remote は無い。
- 操作: Hub WebSocket から production の frontend event `reopen_recent_project` を 5 秒間隔で 10 件送った
  （`p20-drive.mjs`）。project open が ingest を起動し、その continuation で merge scan が 1 回ずつ走る。

### 計測の自己劣化検証（`../README.md` §2）

- 手順 1・2: 記録は merge worker の thread 上で 1 pass に 1 回。GUI thread には何も足していない。
- 手順 3: 1 repo を開いてから 6 分間操作せずに保持し、`route:work_events.ingest` と
  `phase:work_events.ingest.merge_status` がともに open 時の 1 件だけで、待機中に増えないことを確かめた（`p20-idle-samples.jsonl`）。
- 手順 4・5: 省略した。既存 route に phase を 1 つ加え、既存の記録関数を ingest 1 回につき 1 回呼ぶだけで、
  記録は worker 上で行われ、GUI の経路に入らないため。

### 限界

- fixture は最小であり、Work 行・branch・worktree が多い repo の値ではない。下限側に偏る。
- browser-check の hook repair / audit は P02 と同じ理由で実行していない。計測は agent を起動しない。
- 起動ログの error 行は 1 件で、installed gwt が OAuth callback port 8765 を使用中という通知だった。

### ファイル

- `p20-run.sh` / `p20-drive.mjs`: 隔離インスタンスの起動・fixture 作成と、open を送って保持する driver。
- `p20-merge-status-samples.jsonl`: 10 回分の `route:work_events.ingest` と `phase:work_events.ingest.merge_status` の生サンプル。
- `p20-idle-samples.jsonl`: 6 分間の待機観測の生サンプル。
- `p20-environment.txt`: checkout SHA、version、OS、CPU、採取時刻。

## P11 index 構築 / P12 GitHub 同期

### 結論

PM の指示（2026-11-28）で、inventory の定義どおりの起点・終点で再計測した。P11 は暫定値が得られた。
P12 は強制 refresh が 3 回とも失敗で返り、成功した同期の所要時間は得られなかった。

| 行 | 経路 | n | p50 ms | p95 ms | 最悪 ms | 判定（暫定） |
|---|---|---:|---:|---:|---:|---|
| P11 | index 構築（Issues cell） | 3 | 1640.9 | 1071055.9 | 1071055.9 | 判定不能（バジェット未定義） |
| P12 | GitHub 同期（強制 refresh） | 3 | — | — | — | 未測定（成功 0 / 失敗 3） |

p50 / p95 は nearest-rank。n=3 のため p95 は最悪値と同じ。

- P11 の 1 回目（1071 秒）は、fresh HOME の初回 Issue 同期と index coordinator の待ちを含む。
  2・3 回目は 1494〜1641 ms。要求の受付（楽観的な `repair_started_at` の到着）は 3 回とも 2 ms 未満。
- P12 の 3 回はすべて `knowledge_error` で返った。内容は `cached Issue #<n> changed during refresh` で、
  所要時間は 59827 / 2881 / 5516 ms。初回同期（約 15 分、Issue 約 8000 件）と並行して index 側の Issue cache refresh も
  走っており、`crates/gwt/src/issue_cache.rs` は entry を楽観的に書き込み、競合した 1 件で同期全体を失敗にする。
  ユーザーの Refresh が初回同期の間に失敗し続けうる不具合の候補として PM に報告した。

### 計測の起点と終点

- P12 起点: Issue window の `load_knowledge_bridge`（`refresh:true`）が socket に出た時点。app が request id を振り直すため、
  送信側で実際の id を読む。終点: 同じ id への `knowledge_entries`（成功）または `knowledge_error`（失敗）の到着。
- P11 起点: `rebuild_index_cell`（scope `issues`）の送信。終点: その後の `project_index_status` のうち、
  `repair_started_at` が無く `scopes.issues.healthy` が true になった最初のもの。
- どちらも browser の受信時刻であり、画面の描画は含まない。

### 条件

- 対象: checkout `019f8d4594`（P20 計装を含む）の `target/debug/gwt`（debug build、gwtd 9.179.0）。
- host: macOS 27.2.1、Apple M5 Max。2026-11-28T03:34Z〜03:54Z（`sync-index-environment.txt`）。
- 起動: `--no-tray --no-open`、fresh HOME。`~/.gwt/runtime`、`.gitconfig`、`.config`、`.cache`（embedding model）を symlink した。
  GitHub token は環境変数で渡した。PM auto start と Issues の自動実行は無効にした。
- fixture: 空 commit の repo に `origin` として `https://github.com/akiojin/gwt.git` を設定し（fetch / push はしない）、
  その linked worktree を project として開いた。Issue は実 repo の約 8000 件。
- driver: `crates/gwt/playwright/tests/sync-index-inventory-live.spec.ts`（headed Chromium、dark の 1 project）。
  P12 → P11 の順に各 3 回。console / page error は 0 件。

### 限界

- `~/.gwt/runtime` は実ホストと共有しており、index coordinator の待ちは稼働中の fleet と競合する。P11 の値はその条件を含む。
- P12 の外部 API 消費量は未測定。同期は `gh` を起動しないため、`gh` 起動数（各 sample 0）は消費量の代わりにならない。
- 1 project・dark テーマだけで測った。経路は画面描画を含まないため、テーマの差は扱わない。

### P10 Issues scan を測らなかった理由

- 外から scan を要求する経路は `issue.monitor.reconcile` と agent listener の `agent_issue_monitor_scan_now` だけで、
  どちらも agent Session の資格（`GWT_PANE_WS_URL`）を必要とする。
- 実 repo に対する scan は Monitor が無効でも `issue-settlement` 段で merge 済み Issue の close などを提案し、
  executor が GitHub を変更しうる。隔離 HOME からでもリポジトリ外への副作用になるため実行しなかった。
- inventory の暫定値（`docs/perf/issue-6979/`、終点が durable state commit）は境界差ありのまま残す。

### ファイル

- `sync-index.json`: P12 / P11 の生サンプル（outcome、error、受付時間を含む）。
- `sync-index-environment.txt`: checkout SHA、version、OS、CPU、採取時刻。

## P03 / P04 / P06 production route（#8040 の計装修復後）

### 結論

Issue #8040（PR #8060）で production の計装が戻ったため、隔離インスタンスで操作を駆動し、
production が perf log に書いた `route:*` のサンプルをそのまま値として採った。headed Chromium の dark / light で各 20 操作。

| 行 | 経路 | 記録数 / 操作数 | p50 ms | p95 ms | 最悪 ms | バジェット ms | 判定（暫定） |
|---|---|---:|---:|---:|---:|---:|---|
| P03 | project switch | 40 / 40 | 95.0 | 111.2 | 121.3 | 100（ui_response） | p95 超過 |
| P04 | pane 生成（Shell） | 38 / 40 | 38.803 | 50.971 | 56.452 | 5000（pane.create） | 予算内 |
| P06 | prompt 送信 | 40 / 40 | 0.036 | 0.062 | 0.127 | 100（ui_response） | 予算内（handler 区間のみ） |

テーマ別（p50 / p95 / 最悪 ms）:

| 行 | dark | light |
|---|---|---|
| P03 | 95.8 / 111.2 / 121.3 | 91.8 / 110.7 / 118.9 |
| P04 | 43.969 / 56.452 / 56.452（19 / 20） | 31.563 / 47.136 / 47.136（19 / 20） |
| P06 | 0.035 / 0.053 / 0.062 | 0.037 / 0.062 / 0.127 |

p50 / p95 は nearest-rank。

- P03 は `/p/<A>` から `/p/<B>` へのページ遷移で、フロントエンドが遷移開始から最初の workspace frame までを
  `startup_first_frame`（`project_switch:true`）として送り、backend が `route:project.switch` に記録した値。
  #3700 の ui_response 100 ms に対し、p50 は予算内、p95 は超過。
- P04 は Shell window の作成（定義のうち「同期 process 生成」側、`launch_configured_shell`）。agent の起動は
  隔離 HOME に agent の資格情報が無いため測っていない。
- P04 は各テーマの実行でちょうど 1 回分の `route:pane.create` が記録されなかった（smoke でも 3 回中 2 件）。
  Shell の作成経路の一部に計装が無い可能性があり、#8040 の観点で PM に報告した。値は記録された 38 件から計算した。
- P06 は Shell への Enter を含む入力の handler 区間（PTY への書き込み完了まで）。GUI の回復は P07 が扱う。

### 条件

- 対象: checkout `a319539f1a`（`origin/develop` の #8060 を含む）の `target/debug/gwt`（debug build、gwtd 9.179.0）。
- host: macOS 27.2.1、Apple M5 Max。2026-11-28T04:07Z〜04:10Z（`routes-environment.txt`）。
- 起動: `--no-tray --no-open`、fresh HOME。`~/.gwt/runtime`、`.gitconfig`、`.config`、`.cache` を symlink した。
  PM auto start と Issues の自動実行は無効にした。
- fixture: 空 commit の repo の linked worktree を 2 つ開いた。GitHub remote は無い。
- driver: `crates/gwt/playwright/tests/route-inventory-live.spec.ts`。P04 → P06 → P03 の順に各 20 操作。
  P06 は Shell 作成後に owner の attach grace（15 秒）を待ってから送った。console / page error は 0 件。

### 限界

- fixture は最小の repo であり、下限側に偏る。
- P03 の起点はブラウザの遷移開始であり、ユーザーのクリックから遷移開始までは含まない。
- browser-check の hook repair / audit は P02 と同じ理由で実行していない。計測は agent を起動しない。

### ファイル

- `routes-dark.json` / `routes-light.json`: テーマごとの生サンプル、操作数、記録数。
- `routes-environment.txt`: checkout SHA、version、OS、CPU、採取時刻。

## 計測できなかった行

### P19（worktree picker 列挙）

経路が存在しない。File Tree surface は 2026-10-16 の `d2f04c7518`（廃止サーフェスの Rust 表示経路を削除）で取り除かれ、`WindowPreset::FileTree` は `is_removed_legacy` になっている。PM の承認（2026-11-28）により inventory の主表から除外した。
