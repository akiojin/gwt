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
  各 repo の PM auto start と Issue Monitor は browser-check の seed で無効にした。
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
