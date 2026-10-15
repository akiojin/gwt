# Issue #5489: 実 fleet の census baseline

この資料は実稼働 GUI と実 Session/runtime ファイルの読み取り記録である。GUI・agent の再起動、設定変更、プロセス停止は行っていない。依存統合前（commit `a96ce624e`）では census 単体が約 81% 減ったが、実 fleet を読む隔離 Monitor worker 全体は約 35% 減で、半減には届かなかった。Issue #5498 / Issue #5499 統合後の隔離 worker は CPU 中央値で 89.83%、平均使用コアで 67.14% 減となり、この比較条件で半減を確認した。配布版 GUI の修正後 counter と隔離 worker の値を区別する。

## 対象

配布版 GUI は PID `32708`、起動時刻 `2026-10-15T06:33:37.3790587Z`、version `9.121.1`。実行ファイル SHA-256 は `3d35cf1b0264d3e6c7176fd5bc4757b11b730fdd47297e79b0d04ffb9aae5243`。`~/.gwt/session.json` の現在の tabs は次の実 5 root で、Issue #5385 の観測対象と一致する。

- `E:/gwt`
- `E:/160-Idina`
- `E:/KeibaSuite`
- `E:/unity-cli`
- `E:/llmlb`

旧 census source は commit `7e594c0cbca0f84d888f685002b822424ae36859` の `crates/gwt/src/session_inventory.rs`。保存した `target/issue-5489-baseline/session_inventory.rs` の Git blob は `d780e7819a4e23464e6b9b467e2d5816fff27f91` で、調査時の `origin/develop` の同ファイルとも一致した。これは source の比較基準であり、配布版 GUI の build commit を意味しない。

依存統合前の 3 probe は、CI の Clippy 指摘に対する `&matches → matches` の修正後 source で取得した。census 比較の開始は `2026-10-15T08:13:44.115Z`、対象 `session_inventory.rs` の SHA-256 は `82e11229432f8dddabcf19c7da7fccbf173dd0d46e22849ae7bbd3edaa69fc26`。この再取得時は gwt の観測 sessions が 11 件だった。下記の本番 GUI baseline と既存 Issue #5385 probe は以前の時刻の一次記録として維持し、この改善率には使用しない。

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

[comparison.jsonl](comparison.jsonl) の 3 pair で、追加 instrumentation 無しの tick CPU 中央値は `3.781250 → 0.734375 秒`。after/before は `0.194215`、減少率は `80.58%`。各 root の sessions / uncertainties 件数は全 pair で一致した（gwt `11 / 5`、その他各 `1 / 5`）。

[breakdown.jsonl](breakdown.jsonl) は別実行の instrumentation 付き記録。CPU は計測 overhead を含むため改善率には使用しない。各モード 3 回の中央値と呼出し数を記す。

| census 内の処理 | before 呼出し数/tick | after 呼出し数/tick | before CPU 秒 | after CPU 秒 |
| --- | ---: | ---: | ---: | ---: |
| runtime sidecar load | 17,250 | 3,450 | 1.671875 | 0.328125 |
| Session ledger load | 1,615（1,609〜1,624） | 319（319〜322） | 0.468750 | 0.093750 |
| OS 全 process snapshot (`All`) | 10 | 1 | 0.250000 | 0.031250 |
| legacy 等の個別 PID refresh (`Some`) | 4 | 1 | 0.062500 | 0.031250 |

`os_process_refresh` は All + Some の合計で、scoped 項目と重複する集計である。これらの CPU 秒は重ねて合計しない。Session ledger load 数は生きている sidecar/Session の変動で揺れる。runtime sidecar と global OS snapshot の root ごとの反復は 1 census/tick へまとまった。

## 実 Monitor worker の隔離比較

[worker-probe.rs](worker-probe.rs) は実 `monitor_duplicate_runtime::spawn` の新旧 source を診断 binary に組み込み、別 process で 3 tick ずつ実行する。`snapshot` callback から同じ worker 自身の `GetThreadTimes(GetCurrentThread())` を読み、次の callback との差分を前 tick の thread CPU とする。callback 間には製品と同じ 5 秒 sleep が入る。`thread_cpu_seconds` は tick の処理量、`thread_cores` は sleep 込みの平均使用コアである。

5 root の実 prefs を最初に一度だけ読み、[worker-prefs-manifest.json](worker-prefs-manifest.json) の SHA-256 で固定した共通 fixture を用意した。before/after の各子 process は fresh HOME / USERPROFILE に同じ bytes を copy して起動する。全 prefs 書込先がその隔離 HOME 配下であることを起動前に assert する。実 sessions directory は絶対パスで読取り、symlink は作らない。登録 PTY は常に空なので、duplicate-stop / park の group も空である。

```powershell
pwsh -NoProfile -File docs/perf/issue-5489/run-probe.ps1 -Worker -OutputName worker-comparison.jsonl
```

生記録は [worker-comparison.jsonl](worker-comparison.jsonl)、新旧 census / Monitor source の SHA-256 は [context](worker-comparison.jsonl.context.json)。今回の worker 計測開始は `2026-10-15T08:14:48.747Z`。compile 時に source hash を記録し、実行後に現在の source・temporary copy・記録 hash の一致を照合した。before/after の各 prefs hash も固定 manifest の 5 件と一致した。

| 条件 | thread CPU 秒/tick 3 点 | 中央値 | 合計 CPU 秒 / 合計経過秒 |
| --- | --- | ---: | ---: |
| before（PID 13224 / TID 2244） | 7.515625 / 7.000000 / 7.109375 | 7.109375 | 0.485075 コア |
| after（PID 30756 / TID 18792） | 4.890625 / 4.640625 / 4.640625 | 4.640625 | 0.378149 コア |

tick CPU 中央値は `34.73%` 減、平均使用コアは `22.04%` 減。この実測では **worker 全体の CPU 半減は未達**。census 以外の publish/counts/prefs 等も実 worker に含まれ、それらの内訳をこの run だけで特定したとは主張しない。Issue #5499 の prefs 経路調査・修正と組み合わせた再観測へ、この一次記録を引き継ぐ。

## Issue #5498 / Issue #5499 統合後の再測定

統合対象は `origin/develop` の `4cb8fe4a0`。Issue #5498 / PR #5511 の decoded sidecar cache と Issue #5499 / PR #5508 の prefs / publish 最適化を含む。既存の 80.58% / 34.73% は依存統合前の一次記録としてそのまま保持し、新しい結果は `integrated-*` の raw・context・prefs manifest へ分ける。

旧 census / Monitor の baseline module は以前と同じものを使い、after module は統合後 source を使う。両 mode が共有する外部 helper は、統合後に build した同じ gwt library から link する。したがってこの比較は統合後の依存を共通前提にした旧 module と統合後 module の比較であり、Issue #5489 単独の改善率や、旧 run と新 run を横断した改善率とは扱わない。sidecar cache の warm 状態も 3 pair に含まれる。

commit `598b8f0ff5c1222d2c26f684269ce828f5218c2d` の統合済み library `libgwt-61842958423a5674.rlib` を使い、process 内だけ `RUSTUP_TOOLCHAIN=1.99.0` を設定して 3 probe を順に実行した。library の SHA-256 は `10aef7cbb35194f996f997dbb14b1e8e3344c5bbc1c23ea6df81cafb4306f6c4`。各 context に source commit、依存 base commit、census / Monitor / shared prefs source hash と library digest を保存した。旧 raw と manifest は変更していない。

```powershell
$env:RUSTUP_TOOLCHAIN = '1.99.0'
pwsh -NoProfile -File docs/perf/issue-5489/run-probe.ps1 -GwtLibraryName libgwt-61842958423a5674.rlib -DependencyRef 4cb8fe4a0 -OutputName integrated-comparison.jsonl
pwsh -NoProfile -File docs/perf/issue-5489/run-probe.ps1 -GwtLibraryName libgwt-61842958423a5674.rlib -DependencyRef 4cb8fe4a0 -Instrument -OutputName integrated-breakdown.jsonl
pwsh -NoProfile -File docs/perf/issue-5489/run-probe.ps1 -GwtLibraryName libgwt-61842958423a5674.rlib -DependencyRef 4cb8fe4a0 -Worker -OutputName integrated-worker-comparison.jsonl -PrefsManifestName integrated-worker-prefs-manifest.json
```

実 5 root は上記と同じ `E:/gwt` / `E:/160-Idina` / `E:/KeibaSuite` / `E:/unity-cli` / `E:/llmlb`。今回は各 census pair で sessions / uncertainties が一致し、gwt `12 / 5`、その他各 `1 / 5` だった。依存統合前の gwt sessions 11 件という条件差は保持する。

[integrated-comparison.jsonl](integrated-comparison.jsonl) の CPU 中央値は `3.265625 → 0.546875 秒/tick`（`83.25%` 減）。after の初回 cold 値は `1.015625 秒`、後続の warm 値は `0.531250 / 0.546875 秒` で、cold 観測も削除していない。

[integrated-breakdown.jsonl](integrated-breakdown.jsonl) の内訳は次のとおり。CPU 欄は instrumentation 付き 3 回の中央値で、改善率の判定には使わない。

| census 内の処理 | before 呼出し数/tick | after 呼出し数/tick | before CPU 秒 | after CPU 秒 |
| --- | ---: | ---: | ---: | ---: |
| runtime sidecar decode | 17,455 | cold 3,486 / warm 1, 3 | 1.906250 | 0.000000 |
| Session ledger load | 1,775（1,761〜1,807） | 347（346〜353） | 0.484375 | 0.125000 |
| OS 全 process snapshot (`All`) | 10 | 1 | 0.234375 | 0.015625 |
| 個別 PID refresh (`Some`) | 4 | 1 | 0.093750 | 0.062500 |

warm 時の decode CPU のゼロは OS CPU counter の分解能による値であり、処理時間が厳密にゼロという意味ではない。decoder cache に再利用されても、liveness を見る OS snapshot は各 tick で新しく取る。

[integrated-worker-comparison.jsonl](integrated-worker-comparison.jsonl) は、[新しい manifest](integrated-worker-prefs-manifest.json) の同一 5 prefs を before / after の fresh HOME に複製した結果である。登録 PTY と stop / park group は各 0、実 sessions は読み取りのみ。前後とも named worker `gwt-monitor-runtime` の同一 thread から 3 tick を採取した。

| 条件 | thread CPU 秒/tick 3 点 | 中央値 | 合計 CPU 秒 / 合計経過秒 |
| --- | --- | ---: | ---: |
| before（PID 7360 / TID 43384） | 8.593750 / 8.453125 / 8.437500 | 8.453125 | 0.366187 コア |
| after（PID 27268 / TID 45844） | 1.640625 / 0.859375 / 0.812500 | 0.859375 | 0.120336 コア |

統合後 worker の CPU 中央値は `89.83%` 減、平均使用コアは `67.14%` 減で、**この実 fleet を読む隔離 worker の前後比較では半減を確認した**。最初の after tick の warm-up コスト `1.640625 CPU 秒` も 3 点に含む。これは Issue #5498 / Issue #5499 を含む統合条件での結果であり、Issue #5489 単独の成果へ帰属させない。配布版 GUI 自体の差替え・修正後 60 分観測は行っていない。

`node docs/perf/issue-5489/verify.mjs` は旧証跡を `git show a96ce624e` の source hash と照合し、新証跡を現在の source hash と照合する。統合後には同一 library digest、依存 commit の祖先関係、前後の 5 prefs hash、census / worker CPU 中央値と worker 平均コアの半減も検査する。

## 配信済み source と実 GUI の版

PR #5507 は `2026-10-15T12:45:56Z` に merge され、merge commit は `2b6ca0eca084db53ed381a09a85c7d8eb4e2acf0`、最終 head は `046b38b98d4ed238a9e06abacdca51a619f3b743`。再実行時の `origin/develop` にこの commit と既存の全 producing commit が含まれ、branch に未配信 source は無かった。AC-1 の census 内訳と AC-2 の回帰は配信済み source の証拠として扱う。同 head の [Test (Rust)](https://github.com/akiojin/gwt/actions/runs/38732203668/job/116423867533)、[Test (Rust, Windows)](https://github.com/akiojin/gwt/actions/runs/38732203668/job/116423694434)、[coverage / Rust Coverage](https://github.com/akiojin/gwt/actions/runs/38732203522/job/116423167416) はすべて SUCCESS。これは配送 head の既存 CI 証跡であり、今回の session での fresh 実行とは区別する。

稼働 GUI の `v9.123.1` は tag commit `fcd7823b0f1f99cf057775c1e8966edb62786964`（`12:03:44Z`）で、上記 merge より前である。同 tag の `reconcile_project` には root ごとの `observe_sessions` が残る。`git merge-base --is-ancestor 2b6ca0eca v9.123.1` は exit 1。GUI が更新されたという事実だけでは今回の修正後とは判定できない。

[deployed-v9.123.1-baseline.json](deployed-v9.123.1-baseline.json) は `13:07:54Z` に開始した追加の修正前記録。PID `6372`、TID `7604` の `gwt-monitor-runtime` で、`GetThreadDescription` と保持した同一 thread handle の `GetThreadTimes` を使った。終了した thread は観測失敗とし、TID の再利用を別 thread の CPU として数えない。30.9205409 秒で Monitor CPU は 9.9375 秒、0.321388 コア。GUI 全体は 2.148147 コア、implementation agent は区間前後とも 12 窓、runtime は 375 namespaces / 3,812 files。既存 baseline の 11 窓・699 namespaces との条件差を残し、この時点間の差を修正の改善率には使わない。

collector は今後の観測に release tag / commit と fix commit の祖先関係も保存する。`verify.mjs` の隔離 worker 出力は `isolated worker threshold met` と表記し、実 GUI の AC-3 達成を意味する出力にしない。実 GUI の修正後記録は、fix commit を含む配布版で取得してから判定する。

`v9.123.2` は `13:40:39Z` に公開され、tag は修正を含み、対象 census / Monitor source も checkout と一致する。再実行環境の GUI 自動適用設定は root の `auto_apply_updates = true`（canonical `update.settings.status` でも確認）。別の `[agent].auto_update = false` は外部 CLI maintenance の設定であり、GUI 更新を無効にする値ではない（`app_runtime/agent_maintenance.rs` の `start_agent_auto_update`）。

[deployment-wait.json](deployment-wait.json) は更新 cache と各実 5 root の拒否原記録を保存したもの。`13:55:31Z` に `v9.123.2` を検出したが、`13:55:35Z` に `pending_refused`、`Administrator privileges required; the update stays staged` となった。管理者権限での適用を agent が代行せず、Board escalation `206976b5-a230-4c76-adf7-5674d7108ca5` / Issue #5489 comment `6193825907` で PM に調整を引き継いだ。設定・GUI 停止の操作は行っていない。**実 GUI の修正後 CPU は未測定、AC-3 は未チェックである。**

`node docs/perf/issue-5489/verify.mjs` は配信済みの census / 隔離 worker 証拠と新しい修正前 raw の整合性を検証する。after を指定しない成功は AC-3 の完了を意味せず、`PENDING AC-3` を出力する。`node docs/perf/issue-5489/verify.mjs --deployed-after` は after 記録なしで失敗し、版への fix 包含・同じ実 5 root・Monitor 平均コア半減をすべて要求する。

## 更新後の実 GUI 比較（2026-10-15T14:19Z）

PM による通常更新後、Velden が既存 collector を読み取り専用で実行し、[deployed-gui-after.json](deployed-gui-after.json) を取得した。GUI は PID `36240`、起動 `14:11:46.6269894Z`、version `9.123.2`、release commit `4c909ebe1e428cf668f76142e46eabda0b3c4baf`。collector と verifier の双方が、release tag に fix commit `2b6ca0eca084db53ed381a09a85c7d8eb4e2acf0` が含まれることを確認した。GUI・agent の起動、停止、設定変更はこの観測では行っていない。上記の更新拒否は更新前の履歴であり、現時点の blocker ではない。

| 条件 | 修正前 v9.123.1 | 修正後 v9.123.2 |
| --- | ---: | ---: |
| 観測開始（UTC） | 13:07:54 | 14:19:44 |
| 観測時間（秒） | 30.9205409 | 30.2322192 |
| named Monitor thread CPU（秒） | 9.9375 | 4.109375 |
| Monitor 平均使用コア | 0.3213883 | 0.1359270 |
| GUI 全体の平均使用コア | 2.1481473 | 2.0544101 |
| implementation agent 窓（区間前後とも） | 12 | 8 |
| runtime namespaces / files | 375 / 3,812 | 375 / 3,867 |

実 5 root は前後で完全一致し、保持した同一 thread handle で測った `gwt-monitor-runtime` の平均使用コアは **57.71% 減**だった。`node docs/perf/issue-5489/verify.mjs --deployed-after` はこれを含む全 assertion に PASS した。AC-3 の実 fleet の半減を直接観測した証拠であるが、窓数、ファイル数、経過時刻と他の配布済み修正が異なるため、Issue #5489 単独の因果効果とは主張しない。同条件での因果比較は上記の交互 census / 隔離 worker 証拠で補完する。

独立レビューで、verifier が観測時刻を検査せず、after を GUI 起動前の時刻へ変えても PASS することを再現した（RED）。日時の parse 可能性、GUI 起動後の観測、baseline より後の after という assertion を追加し、不正な時系列の拒否と元の raw の PASS を確認した（GREEN）。raw は検査前後で同一 bytes に復元した。

今回の最小 Plan は、Goal: 未配信の collector / baseline と実 GUI after を検証・配信すること、Acceptance criteria: 修正版の包含・実 5 root の一致・named Monitor CPU 半減を専用 verifier で再検査し、fresh `verify.run` と必須 CI 後に develop へマージすること。Non-goals / What stays untouched: runtime の追加最適化、GUI の設定・再起動、別 owner の修正、Issue #5385 の 60 分計測。新しい probe や runtime 修正を加える案と比較し、既存 collector と verifier の再利用を採用した。source の根本修正は既に配信済みで、残る受け入れ要件は実 GUI の観測だからである。

Issue #5385 の 7〜8 窓 / 60 分再測定にはこの raw と版包含証明を引き継ぐ。GUI 全体はなお約 2.05 コアであり、family 全体の平均 1 コア未満という予算達成は主張しない。AC の確定は今回の証跡も `origin/develop` に含まれることを確認してから行う。

## 比較の限界と引継ぎ

- Issue #5385 の元資料は `E:/gwt/work/issue-5385/docs/perf/issue-5385/{context.json,threads.json,census-probe.rs,census-probe.txt,collect.ps1,samples.jsonl,report.md}` にある。他 worktree のファイルは変更していない。
- Issue #5385 の初回 thread 観測は Monitor `8.0625 CPU 秒 / 15.1019369 秒 = 約 0.534 コア`。当時の runtime は 678 namespaces / 4,036 files、実装 agent 8 窓だった。現在は namespace 数・窓数とも増えており、異なる時刻の絶対値を直接改善率にしない。
- source の改善率は同じ profile・同じ binary・同じ実 root で交互採取する。各 root の sessions / uncertainties も保存し、fleet の変動を確認する。
- library census 単体の CPU 半減は、本番 `gwt-monitor-runtime` thread の修正後 counter を直接測った証拠ではない。隔離 worker は依存統合前に半減未達、統合後の同一 fixture 比較で半減を確認した。配布版 GUI の修正後 counter と Issue #5385 の 7〜8 窓 / 60 分は別の観測対象として扱う。

User Verification Result: n/a (autonomous)

Agent Visual Check: n/a (no UI surface; read-only performance evidence)
