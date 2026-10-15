# Issue #5489: 実 fleet の census baseline

この資料は実稼働 GUI と実 Session/runtime ファイルの読み取り記録である。GUI・agent の再起動、設定変更、プロセス停止は行っていない。census 単体は約 80% 減ったが、実 fleet を読む隔離 Monitor worker 全体は約 40% 減で、半減には届かなかった。配布版 GUI の修正後 counter と隔離 worker の値を区別する。

## 対象

配布版 GUI は PID `32708`、起動時刻 `2026-10-15T06:33:37.3790587Z`、version `9.121.1`。実行ファイル SHA-256 は `3d35cf1b0264d3e6c7176fd5bc4757b11b730fdd47297e79b0d04ffb9aae5243`。`~/.gwt/session.json` の現在の tabs は次の実 5 root で、Issue #5385 の観測対象と一致する。

- `E:/gwt`
- `E:/160-Idina`
- `E:/KeibaSuite`
- `E:/unity-cli`
- `E:/llmlb`

旧 census source は commit `7e594c0cbca0f84d888f685002b822424ae36859` の `crates/gwt/src/session_inventory.rs`。保存した `target/issue-5489-baseline/session_inventory.rs` の Git blob は `d780e7819a4e23464e6b9b467e2d5816fff27f91` で、調査時の `origin/develop` の同ファイルとも一致した。これは source の比較基準であり、配布版 GUI の build commit を意味しない。

## 本番 process/thread counter

[collect-baseline.ps1](collect-baseline.ps1) は同一 PID と開始時刻を固定し、`Process.TotalProcessorTime` と各 `ProcessThread.TotalProcessorTime` の差を読む。経過時間は `Stopwatch`、コア相当は CPU 秒 / 実経過秒とし、論理 CPU 数では割らない。thread counter は順に読むため短い区間の境界誤差を残す。窓一覧は区間前後のみ採取する。

| 記録 | 開始 UTC | 実経過秒 | GUI CPU 秒 | GUI コア相当 | Monitor CPU 秒 | Monitor コア相当 |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| [baseline-followup.json](baseline-followup.json) | 2026-10-15T07:28:27.792Z | 30.0257424 | 69.609375 | 2.31832 | 14.78125 | 0.49229 |

Monitor は TID `36776`。同一 GUI identity の Issue #5385 `threads.json` が、この TID の OS thread 名を `gwt-monitor-runtime` と記録している。今回の .NET collector 自体は thread 名を再取得していない。

この区間は下記の既存 census probe 終了後に採取した。区間前後は implementation agent `11 → 11` 窓、runtime は `699 namespaces / 4,064 files / 1,919,993 bytes`。測定中に他 agent の build 等が進んでおり、host の完全な無負荷状態を主張しない。

再採取は新しい出力名で実行する。

```powershell
pwsh -NoProfile -File docs/perf/issue-5489/collect-baseline.ps1 -GuiProcessId 32708 -DurationSeconds 30 -OutputName another-baseline.json
```

## 既存 read-only census probe の再実測

Issue #5385 の既存 binary `%TEMP%/gwt-census-probe-5385.exe` を使い、通常の Windows HOME と上記 5 root で再実行した。binary/source の情報は [census-before-context.json](census-before-context.json)、生出力は [census-before.txt](census-before.txt)。probe の改変・再 build は行っていない。

```powershell
& "$env:TEMP/gwt-census-probe-5385.exe" "$env:USERPROFILE/.gwt/sessions" E:/gwt E:/160-Idina E:/KeibaSuite E:/unity-cli E:/llmlb
```

| pass | 5 root の CPU 秒合計 | machine census の CPU 秒 | 観測 sessions |
| --- | ---: | ---: | --- |
| 1 | 3.843750 | 0.875000 | gwt 12 / その他各 1 / machine 16 |
| 2 | 3.515625 | 0.968750 | gwt 12 / その他各 1 / machine 16 |

これは既存 dev profile の library にリンクした診断プログラムの値である。配布版 GUI の thread CPU 秒へ換算しない。

## 同一 library 条件での比較手順

[probe.rs](probe.rs) と [run-probe.ps1](run-probe.ps1) は baseline/current の census module を一時 copy して同じ診断 binary に組み込み、実 5 root に対して 3 pair の before/after を交互順序で実行する。旧実装は `observe_sessions` を root ごとに呼ぶ。新実装は `ProjectSessionCensus::capture` を 1 回呼び、root ごとの `observe` と capture・tick 合計を別々に測る。Session/runtime ファイルは書き換えない。

既に checkout library が build 済みで、新 API が公開された後に次を実行する。script が compile するのは診断プログラムだけで、cargo build は実行しない。

```powershell
pwsh -NoProfile -File docs/perf/issue-5489/run-probe.ps1
pwsh -NoProfile -File docs/perf/issue-5489/run-probe.ps1 -Instrument -OutputName breakdown.jsonl
```

baseline module が無い fresh checkout では、上記 commit の source を `target/issue-5489-baseline/session_inventory.rs` に復元してから実行する。製品 source は変更しない。

最初の実行は改善率用の追加 instrumentation 無しの値である。`-Instrument` は一時 copy だけで runtime sidecar load、Session ledger load、`sysinfo::refresh_processes_specifics` を wrapper に置き換え、呼出し数・CPU 秒・wall 秒を記録する。個々の load に `GetProcessTimes` を追加する計測コストがあるため、instrumented CPU を改善率の根拠にしない。wrapper は census module 内のこれらの呼出しを数えるもので、gwt 全体の OS API 呼出し総数ではない。

## 同条件の census 比較結果

[comparison.jsonl](comparison.jsonl) の 3 pair で、追加 instrumentation 無しの tick CPU 中央値は `3.187500 → 0.640625 秒`。after/before は `0.200980`、減少率は `79.90%`。各 root の sessions / uncertainties 件数は全 pair で一致した（gwt `12 / 5`、その他各 `1 / 5`）。

[breakdown.jsonl](breakdown.jsonl) は別実行の instrumentation 付き記録。CPU は計測 overhead を含むため改善率には使用しない。各モード 3 回の中央値と呼出し数を記す。

| census 内の処理 | before 呼出し数/tick | after 呼出し数/tick | before CPU 秒 | after CPU 秒 |
| --- | ---: | ---: | ---: | ---: |
| runtime sidecar load | 17,115 | 3,423 | 1.671875 | 0.312500 |
| Session ledger load | 1,536（1,534〜1,677） | 308（303〜309） | 0.500000 | 0.093750 |
| OS 全 process snapshot (`All`) | 10 | 1 | 0.234375 | 0.015625 |
| legacy 等の個別 PID refresh (`Some`) | 4 | 1 | 0.062500 | 0.031250 |

`os_process_refresh` は All + Some の合計で、scoped 項目と重複する集計である。これらの CPU 秒は重ねて合計しない。Session ledger load 数は生きている sidecar/Session の変動で揺れる。runtime sidecar と global OS snapshot の root ごとの反復は 1 census/tick へまとまった。

## 実 Monitor worker の隔離比較

[worker-probe.rs](worker-probe.rs) は実 `monitor_duplicate_runtime::spawn` の新旧 source を診断 binary に組み込み、別 process で 3 tick ずつ実行する。`snapshot` callback から同じ worker 自身の `GetThreadTimes(GetCurrentThread())` を読み、次の callback との差分を前 tick の thread CPU とする。callback 間には製品と同じ 5 秒 sleep が入る。`thread_cpu_seconds` は tick の処理量、`thread_cores` は sleep 込みの平均使用コアである。

5 root の実 prefs を最初に一度だけ読み、[worker-prefs-manifest.json](worker-prefs-manifest.json) の SHA-256 で固定した共通 fixture を用意した。before/after の各子 process は fresh HOME / USERPROFILE に同じ bytes を copy して起動する。全 prefs 書込先がその隔離 HOME 配下であることを起動前に assert する。実 sessions directory は絶対パスで読取り、symlink は作らない。登録 PTY は常に空なので、duplicate-stop / park の group も空である。

```powershell
pwsh -NoProfile -File docs/perf/issue-5489/run-probe.ps1 -Worker -OutputName worker-comparison.jsonl
```

生記録は [worker-comparison.jsonl](worker-comparison.jsonl)、新旧 census / Monitor source の SHA-256 は [context](worker-comparison.jsonl.context.json)。記録済み source hash を追加した時点で、source から作った temporary copy が実行に使った copy と完全一致することも確認した。

| 条件 | thread CPU 秒/tick 3 点 | 中央値 | 合計 CPU 秒 / 合計経過秒 |
| --- | --- | ---: | ---: |
| before（PID 36032 / TID 5892） | 6.609375 / 5.750000 / 6.562500 | 6.562500 | 0.526125 コア |
| after（PID 24852 / TID 35908） | 3.953125 / 3.765625 / 3.984375 | 3.953125 | 0.425164 コア |

tick CPU 中央値は `39.76%` 減、平均使用コアは `19.19%` 減。この実測では **worker 全体の CPU 半減は未達**。census 以外の publish/counts/prefs 等も実 worker に含まれ、それらの内訳をこの run だけで特定したとは主張しない。Issue #5499 の prefs 経路調査・修正と組み合わせた再観測へ、この一次記録を引き継ぐ。

## 比較の限界と引継ぎ

- Issue #5385 の元資料は `E:/gwt/work/issue-5385/docs/perf/issue-5385/{context.json,threads.json,census-probe.rs,census-probe.txt,collect.ps1,samples.jsonl,report.md}` にある。他 worktree のファイルは変更していない。
- Issue #5385 の初回 thread 観測は Monitor `8.0625 CPU 秒 / 15.1019369 秒 = 約 0.534 コア`。当時の runtime は 678 namespaces / 4,036 files、実装 agent 8 窓だった。現在は namespace 数・窓数とも増えており、異なる時刻の絶対値を直接改善率にしない。
- source の改善率は同じ profile・同じ binary・同じ実 root で交互採取する。各 root の sessions / uncertainties も保存し、fleet の変動を確認する。
- library census 単体の CPU 半減は、本番 `gwt-monitor-runtime` thread の修正後 counter を直接測った証拠ではない。実 fleet を読む隔離 worker の測定は上記のとおり半減未達だった。Issue #5489 AC-3 の worker 全体半減と Issue #5385 の 7〜8 窓 / 60 分は、それぞれ対象版・対象条件での追加観測と区別して扱う。

User Verification Result: n/a (autonomous)

Agent Visual Check: n/a (no UI surface; read-only performance evidence)
