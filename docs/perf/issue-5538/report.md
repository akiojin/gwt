# Issue #5538: historical real-fleet baseline

これは **配布版 v9.121.1 の過去の実 fleet 観測**を旧 handler 警告と対応づけた baseline であり、Issue #5538 の新しい計測器を使った修正後の結果ではない。**この baseline 単独では AC-3 を証明しない。**

集計区間は UTC **2026-10-15T06:45:46.6757267Z 以上、2026-10-15T07:15:46.6757267Z 未満**の厳密な 1,800 秒。元の [Issue #5385 の実測](../issue-5385/report.md)と [samples.jsonl](../issue-5385/samples.jsonl) の minute 0〜31、32 点が区間を挟む。全点で同じ GUI PID `32708` / 起動時刻 `2026-10-15T06:33:37.3790587Z`、同じ project の `running` / `implementation_agent` の Codex 8 窓と別枠の PM 1 窓を記録している。minute 0→30 の単調時計の差は `1799.9882954` 秒なので、それを 1,800 秒に丸めて証拠にせず、minute 31 まで含めて区間末尾を挟んだ。

[fleet-baseline.json](fleet-baseline.json) は元の context / samples の SHA-256、抽出した pane roster・全 32 点の窓数証拠、実 `codex.exe` 8 プロセスの census、271 件の匿名化した旧 slow-dispatch 原記録を保存する。実ホームのログ `USERPROFILE/.gwt/logs/gwt.log.2026-10-15` は読み取りだけを行い、認証情報、prompt、PTY 出力、他の log fields は転記していない。完全な元ログの SHA-256 とサイズも JSON に記録した。

修正後の実8窓・30分測定は [post-change-report.md](post-change-report.md) に別記する。

## 累積停止時間の順位

旧 `EventLoopDispatchTimer` の `gwt.frontend.timing` / `blocked the GUI event loop` 警告から `elapsed_ms > 50` を選ぶ。旧警告の閾値は 30ms なので、この条件の完了済み slow dispatch は警告側に含まれる。`p95 slow` は **50ms 超の母集団だけ**の nearest-rank 値であり、全 handler の p95 ではない。

| 順位 | 旧 event 種別 | 回数 | 累積 ms | p95 slow ms | 最大 ms |
| --- | --- | ---: | ---: | ---: | ---: |
| 1 | `IssueMonitorCapacityChanged` | 194 | 10,901 | 68 | 80 |
| 2 | `ProviderUsageSnapshot` | 60 | 8,193 | 320 | 334 |
| 3 | `Frontend` | 10 | 5,176 | 791 | 791 |
| 4 | `IssueMonitorDaemonInbox` | 4 | 363 | 143 | 143 |
| 5 | `MainEventsCleared` | 2 | 149 | 76 | 76 |
| 6 | `BoardProjectionRefreshed` | 1 | 92 | 92 | 92 |

合計 **271 件 / 24,874ms**。観測された対象種別は 6 個だけであり、上限 10 件の表を埋めるための種別は作らない。これは slow dispatch の handler 実行時間で、handler の前の queue 待ち時間や CPU 使用時間を足した値ではない。

## 子 Issue の調査開始点

上位の名前はコード上の dispatch 分岐に直接対応する。`IssueMonitorCapacityChanged` は capacity 変更反映・Work projection、`ProviderUsageSnapshot` は usage snapshot 反映を入口に、同じ raw 時刻で下流の同期処理を追える。`IssueMonitorDaemonInbox`、`BoardProjectionRefreshed` も各分岐が入口になる。旧 timer が粗い `Frontend` / `MainEventsCleared` は、新計測器で内側の処理を分離してから root cause を確定する必要がある。この baseline 自体は原因を断定しない。

補助の `frontend_latency_details` は同区間の receive-to-completion 警告 15 件を保存する。`Frontend` の内訳を探す手掛かりは `SetIssueMonitorAutonomousMode` (791ms)、`SetIssueMonitorEnabled` (622 / 742 / 506ms)、`LaunchWizardAction` (698ms)、`SetIssueMonitorAutoRefill` (476 / 423 / 415 / 417ms)、`LoadKnowledgeBridge` (65ms)。`FocusWindow` / `UpdateWindowGeometry` の handler 自体は 0ms でも queue 待ちがあった。これらは outer dispatch と重なるため、上表の累積値へ再加算しない。

## 証拠の限界と修正後の検証

- 元の `pane.list` は観測 project の 8 implementation 窓を示す。GUI には 5 Projects があり、同時刻の frontend 記録は `window_count=21`、process census は Codex 8 個以外に Claude 5 個を含む。GUI 全体が厳密に agent 8 窓だけだったとは証明しない。
- 窓数は 1 分ごとの標本であり、標本間の一時的な窓数変更は排除できない。全 8 窓が全時間帯でコマンド実行中だったとも主張しない。
- roster の記録名は Virelan、Valen、Cevrin、Velrin、Veylan、Veylin、Ceryn。残る `agent-1413` は名前未記録。全 8 窓の `source_session=none` で、provider Session ID は未記録。pane ID・作成時刻は JSON に残したが、名前・Session ID を推定して補完しない。
- 実 provider の PID / 作成時刻の census は 1 回の観測で、pane↔PID 対応は未記録。旧 handler 警告にも PID・起動時刻・pending 件数がない。同じ real HOME と時刻による対応づけであり、各警告を GUI PID に直接結びつける証拠ではない。
- 旧計測器の baseline なので、新実装による per-handler / pending / 分次集計や計測 overhead の確認を代替しない。合成 PTY、再生、fixture の結果を実 agent の結果として扱っていない。

厳密な AC-3 の確認には、新計測器を含む checkout の隔離 HOME で実 provider agent 8 窓を 30 分以上観測し、全体窓数・pane↔provider プロセス・GUI PID を対応づけた証拠が必要になる。今回の historical baseline を、その実施済み証拠として扱わない。

## 再計算

`node docs/perf/issue-5538/verify-fleet-baseline.mjs`

保存した raw の区間・閾値・件数・累積・p95・順位を再計算し、元の Issue #5385 artifact の hash、pane roster、全 32 点、process census と照合する。成功は baseline の整合性を示すだけであり、AC-3 完了を示さない。

User Verification Result: n/a (autonomous)

Agent Visual Check: n/a (historical read-only evidence extraction)
