# Issue #5538: source-matched real eight-agent measurement

実 builtin Codex 8 窓の定常観測は **30分11.503秒、234点、完全な集計区間30個**で成功した。
この heartbeat / private progress receipt 負荷の定常区間では、完了した50ms超の handler は **0件 / 0ms**だった。
したがって定常区間の `top10` は空配列である。空欄を埋めるための種別を作らず、GUI が改善したとも主張しない。

## 観測とソース

- UTC: `2026-10-15T19:46:52.352Z` から `2026-10-15T20:17:03.855Z`。
- GUI PID: `39712`、起動: `2026-10-15T19:40:57.8437078Z`。
- 単調時計: `1811503.1714ms`、完全区間の合計: `1801097.5498ms`。
- 最大標本間隔: `9823.1519ms`。
- checkout の測定前 HEAD: `408a80e3dceff0c7be46df8d6f84e735b1388973`。
- native binary SHA-256: `1135DE39E245887B8C294B464839F9ADFACA5839D9891C7531811BEBBE112EAE`。
- 測定したソースの固定 commit: `500d2847fb2f62ba772ffdf6dc1ddde8968cf5af`。後日のコード変更を、この測定結果と同一視しない。
- 実装は測定時には未コミットだったため、測定前HEADだけをコードの証拠にしない。起動時に採取した本番ソース6ファイルの正規化 Git blob hash を保存し、固定commitのソースとの完全一致を verifier で確認する。後続の develop 統合で変わったファイルは明示して、測定対象をすり替えない。
- 全標本で同じ8個の `codex.exe` の PID / 起動時刻と Session ID、実コマンドの shell PID / 親子関係を確認した。
- 実 tool-result の READY marker が各 Session と shell を結び、その後の稼働は shell 自身の private receipt で確認する。モデル応答の遅れを稼働停止と誤認しない。
- 完全区間の `dropped_windows` は全て0。dark / light の headed Chromium は各1件成功し、console / page error は0件。
- 所有GUIと子プロセスを停止し、一時認証コピーを除去した。cleanup error は0件。

| Fixture | Session（先頭8桁） | provider PID | heartbeat shell PID |
| --- | --- | ---: | ---: |
| 0 | `e17fe878` | 15008 | 4664 |
| 1 | `4240b291` | 40624 | 47956 |
| 2 | `e8d6900f` | 46180 | 39524 |
| 3 | `7d137a90` | 37548 | 39432 |
| 4 | `9d1ef8c3` | 41224 | 22292 |
| 5 | `d6c5514e` | 30664 | 42244 |
| 6 | `009a555d` | 53268 | 34296 |
| 7 | `b48997b7` | 22732 | 15904 |

## 定常30分の上位10

該当0種。30個の typed zero-count liveness summary は存在しており、ログの欠落を0件と読み替えた結果ではない。
分次の count / total / p95 / max / pending は [fleet-post-change.json](fleet-post-change.json) に保存した。
50ms超の母集団だけの p95 であり、全 handler の p95 ではない。分次 p95 を平均して全体 p95 とすることもしない。

## 起動・境界区間の上位10（定常30分の順位には含めない）

同じ新計測器で起動中には実際の50ms超を記録した。ここは provider 数が0から8へ増える区間と観測境界を含み、AC-3 の定常8窓30分の数値に混ぜない。
各行の raw 分次 summary は `warmup_or_boundary_rows` に保持している。

| 順位 | handler | 回数 | 累積 ms | 最大 ms | pending 最大 |
| --- | --- | ---: | ---: | ---: | ---: |
| 1 | `NewEvents` | 17 | 5870.2731 | 532.4765 | 2 |
| 2 | `ProviderUsageSnapshot` | 13 | 5717.7293 | 1789.2385 | 15 |
| 3 | `MainEventsCleared` | 12 | 4707.1352 | 474.3298 | 3 |
| 4 | `RuntimeOutput` | 9 | 3561.7065 | 534.9301 | 10 |
| 5 | `FrontendReady` | 7 | 2468.6196 | 656.3224 | 13 |
| 6 | `IssueMonitorCapacityChanged` | 5 | 1674.0794 | 425.3854 | 9 |
| 7 | `RedrawEventsCleared` | 3 | 1128.3804 | 381.8147 | 7 |
| 8 | `LaunchWizardLaunchMaterializationRequested` | 2 | 905.4091 | 500.4791 | 7 |
| 9 | `RuntimeDaemonEnsureTick` | 3 | 610.8519 | 323.2726 | 3 |
| 10 | `ListIssueMonitor` | 2 | 581.1028 | 471.8206 | 14 |

起動・境界の全13種合計は **76件 / 28270.5689ms**。pending unknown は全て0。

## 子 Issue 用の調査境界

上表は完了した outer callback の計測であり、関数内の原因まで特定した値ではない。共通の output flush / callback 前処理も含む。次の境界と再現条件を添えて PM が子 Issue を切り出せる。

- `NewEvents` / `MainEventsCleared` / `RedrawEventsCleared`: `main.rs` の callback 共通前処理、`flush_runtime_output_batches`、project aggregate 更新を分離して調べる。
- `ProviderUsageSnapshot`: `AppRuntime::handle_provider_usage_snapshot` の全 project の prefs path 解決、`try_mutate_issue_monitor_prefs`、quota candidate sweep を分離して調べる。
- `RuntimeOutput`: `handle_runtime_output_event` と共通 output flush を入口に、8 provider 起動時の batch / terminal 更新を追う。
- `FrontendReady` / `ListIssueMonitor`: 初回 hydration と Monitor read model の同期処理を入口にする。
- `IssueMonitorCapacityChanged`: `issue_monitor_capacity_changed_events` の prefs 読み込み、capacity 観測と status projection を分離する。
- Launch materialization / daemon ensure: 起動時だけの停止として、既存 launch / daemon phase 計測と時刻を対応づける。
- 実作業負荷の旧 baseline は [report.md](report.md) に別記した。そこでは271件 / 24,874ms、上位6種を記録している。定常 heartbeat の0件と比較して性能向上を計算してはならない。

## 条件と限界

- fresh HOME、8個の disposable Git Project、各1個の実 builtin Codex。PM / Issue Monitor の自動起動は無効。識別用 origin は接続しない `.invalid`。
- 実 agent が fixture を読み、40分のコマンドを開始する。コマンドは5秒ごとの heartbeat と private progress receipt だけを生成し、実装・build・Git / GitHub mutation は行わない。
- 通常の実装作業、production の大きな Work / Issue 状態、他 provider の混在を再現した測定ではない。他のホスト負荷も固定していない。
- 実 agent を模した shell provider や録画再生ではない。実 provider のプロセスと tool-result hash を保存し、prompt / terminal本文 / 認証情報は転記しない。
- 起動時の pane naming / Work mutation の authority refusal は別の gwt 摩擦として PM に報告済み。authority は緩和せず、独立して許可された測定コマンドだけを継続した。

## 計測コストと再計算

[collector-cost.json](collector-cost.json) の交互10 roundでは、同じ handler work に対する代表値は baseline 1732.654ns、収集有効1684.606nsだった。
この標本で増加は検出されなかったが、差 -48.048ns は測定ノイズであり高速化の証拠ではない。fast collection path の限定測定であり、whole-fleet CPU や厳密な遅延上限は主張しない。

- `node docs/perf/issue-5538/verify-post-change.mjs` — 固定commitの正規化ソース、234点の8窓 / PID / receipt、30完全区間、順位、headed 両テーマ、cleanup を再計算。
- `node docs/perf/issue-5538/verify-fleet-baseline.mjs` — 過去の実負荷 baseline だけを独立再計算。
- 実測の再実行: `pwsh -NoProfile -File scripts/verify-issue-5538.ps1 -Fleet --headed --trace=on`。
- 短時間の GUI 確認: `pwsh -NoProfile -File scripts/verify-issue-5538.ps1 --headed`。

source が進んだ shallow checkout で固定commitが無い場合は、`git fetch origin 500d2847fb2f62ba772ffdf6dc1ddde8968cf5af` 後に再計算する。

User Verification Result: n/a (autonomous)

Agent Visual Check: pass
