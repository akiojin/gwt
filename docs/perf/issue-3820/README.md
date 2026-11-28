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

## 計測できなかった行

### P19（worktree picker 列挙）

経路が存在しない。File Tree surface は 2026-10-16 の `d2f04c7518`（廃止サーフェスの Rust 表示経路を削除）で取り除かれ、`WindowPreset::FileTree` は `is_removed_legacy` になっている。PM の承認（2026-11-28）により inventory の主表から除外した。

### P20（workspace merge 状態の反映）

外から起動する操作が無い。merge 状態の scan は Work events ingest と tick の continuation として内部で始まり、要求を区切る event も route 計装も無い。値を得るには #3700 の route 計装（scan 要求から反映まで）の追加が必要。PM の裁定（2026-11-28）で今回は計装を追加せず、「未計装・外部起動操作なし」として残し、Phase 2 後半で再評価する。
