# Issue #4172: Windows 入力停止の診断（2026-09-09）

「結論と未確定範囲」から「検証と保全範囲」までは先行診断コミット `ebd29f32d` 時点の記録。
その後の実装・headed測定・PM裁定は末尾の「追加計装と最小修正」に記載する。

## 結論と未確定範囲

稼働中の Windows GUI で、WebSocket 接続を受理した後の `pane.list` 無応答を再現した。
同時刻の main スレッドには、プロセス生成とオブジェクト待機のスタックを採取できた。
`WorkTipSubjects` と `BoardProjectionChanged` の処理がそれぞれ約28秒イベントループを占有していた。
これは再起動約3時間後の観測であり、起動直後の restore だけでは説明できない。

ただし、稼働 release に対応する PDB がなく、`gwt+offset` を内部関数名へ解決できていない。
子 Git の存在、ソース上の同期 Git 経路、main の待機は対応するが、各 Git コマンドの
全所要時間がそのまま main 占有時間だとは断定しない。バックグラウンドの Git も同時に動いている。
4面それぞれの実際のキー入力、ペイン数の制御実験、macOS同手順、受信→受付の恒久計装は未完である。
この診断資料だけで Issue を閉じたり、修正済みと扱ったりしない。

## 対象と測定方法

- 稼働対象: インストール済み `gwt.exe` v9.94.0、PID 34952、main TID `0x9a80`。
- 起動: 2026-09-09 19:17:50.737 JST。以前の PID 24932 は既に終了していた。
- 読んだソース: `0849fd177d82bab401d3201152b7db2cabbe3342`（この checkout）。
  稼働 release と同一ビルドではないため、以下の file:line は経路候補の特定に使用する。
- ログ: `~/.gwt/projects/ffa30e30ecb522d5/logs/gwt.log.2026-09-09`。
- スタック: [採取プログラム](../../scripts/diagnostics/capture-windows-stacks.cpp) から
  Microsoft DbgEng の `DEBUG_ATTACH_NONINVASIVE | DEBUG_ATTACH_NONINVASIVE_NO_SUSPEND` を使用。
  プロセスを停止しないためフレームは実行と競合し、途中で unwind が切れる標本もある。
  停止したプロセスの整合したダンプと同じ精度ではない。
- 全スレッドを28回採取。連続採取区間内の間隔は約2.7秒で、継続的なプロファイラの比率ではない。
  main の抜粋、元ファイルの SHA-256、PID、時刻、API結果を
  [evidence JSON](issue-4172-evidence.json) に保全した。全ダンプのメモリ内容は含めない。
- WPR `-start CPU -filemode` は `0xc5585011`（profile system performance の policy 有効化失敗）。
  ETW trace は採れていない。これを理由に診断を停止せず、上記 DbgEng に切り替えた。

## 実測タイムライン（JST）

| 時刻 | 観測 |
| --- | --- |
| 22:12:30.678 | main は `NtUserGetMessage` / `GetMessageW` 待機。停止証拠ではない。 |
| 22:12:58.504 | `pane.list` 開始、825msで成功。9ペインが返った。全プロジェクトの総数ではない。 |
| 22:13:26.881–54.427 | `BoardProjectionChanged` 27,546ms。開始は終了ログ時刻から elapsed_ms を引いた近似。 |
| 22:13:32.077 / 34.752 / 40.113 / 42.781 | 上記区間内で main は `NtWaitForMultipleObjects`。 |
| 22:13:37.427 | 同区間内で main は `NtReadFile`。 |
| 22:15:09.107–36.921 | `WorkTipSubjects` 27,814ms。 |
| 22:15:22.526 | main に `NtCreateUserProcess` → `CreateProcessInternalW` → `CreateProcessW`。 |
| 22:15:31.384 | `pane.list` 開始。22:15:31.511 に `/internal/pane-ws` HTTP 101。 |
| 22:15:36.970–22:16:04.902 | `BoardProjectionChanged` 27,932ms。 |
| 22:16:02頃 | `pane.list` が `pane_backend_unresponsive`。診断は「受理後15000ms無応答」、コマンド全体は30,893ms。両者を同じ計測区間として扱わない。 |

22:15:00–50 の子プロセス照会では PID 34952 の子 Git を254標本、214個の異なるPIDで捕捉した。
130標本に command line があり、うち8標本が hook config の `ls-files --error-unmatch`、
96標本が `git cherry`、残り26標本がその他だった。重複標本を含むので起動回数ではない。
CIM照会開始時刻とプロセス生成時刻を別フィールドに保存した。照会中に生成・終了するため、
照会開始より後の生成時刻や null command line が含まれる。
JSONには全標本の集計値・元ファイルhashと、command lineを取得できた130標本を保存した。

## index 仮説の切り分け

22:13台の `verify.lease.status` は、GUI PID 34952 が `--issues` の index lease を保持していた。
この記録には `expires_at_ms` がある。過去 PID 24932 の TTLなし観測を現在にも当てはめない。
同じ index lease の取得時刻は22:04台であり、825msで成功した標本と停止標本の両方を含む。
したがって、lease保持はこの停止の十分条件ではない。資源競合の寄与までは否定できない。

ソースでも `gwt-core/src/index/runtime.rs:897` の `gwt-index-issues` と `:1016` の
`gwt-index-issues-runner` は別スレッドである。同一PIDというだけで main 実行とは言えない。

## コード上の同期経路と次の計測点

1. `crates/gwt/src/main.rs:9127` → `app_runtime/board.rs:677` → `:358` の Board milestone transaction。
   `workspace_projection/persistence.rs:3659` → `:3680` の `lock_exclusive()` は同期・期限なし。
   今回の main 標本では `LockFileEx` を確認しておらず、ロックを実原因と断定しない。
2. `board.rs:397` → `title_sync.rs:94` → `workspace_views.rs:3603` の全件 projection。
   `:3704` / `:3723` の hook health が同期実行される。
3. `cli/hook/health.rs:439` → `crates/gwt-skills/src/settings_local.rs:596` は configごとに
   `git ls-files --error-unmatch -z` を起動し、出力を同期取得する。GUI子プロセスにもこのコマンドを捕捉した。
4. `cli/hook/health.rs:282` → `crates/gwt-core/src/error_ledger.rs:198` は
   `errors.*.jsonl` の全ファイルを解析後に日時filterする。Work行ごとの反復I/O候補であり、
   今回はこの部分の時間内訳を測っていない。
5. 通常の端末入力は `embedded_server.rs:4253` / `:4316` でPTYへ直接書き込む。
   echoは `app_runtime/pty_io.rs:1766` → `main.rs:9084` を通る。
   `pane.list` 無応答だけを「全端末の入力バイトが受付されていない」証拠にはしない。
6. restoreの実spawnは `app_runtime/launch.rs:4514` の別スレッド。
   `startup.rs:1027` / `:1040` のPM refreshなど同期部分と分ける。

既存の `gwt.frontend.timing` は `main.rs:401` のDrop時に出るため、未復帰の停止はまだ記録されない。
次の恒久計装では受信時刻を保持し、queue wait / handler / PTY受付 / echoを分離する。
全件hook health内訳とWork件数を同時計測してから、最小の修正を判断する（SPEC #1935 tasks参照）。

## 再現・採取手順とAC-1の扱い

1. 対象のPID、起動時刻、version、binary hashを記録する。過去のPIDを再利用しない。
   今回と同程度のWork蓄積がある対象で、通常のBoard/Work更新とバックグラウンド更新を観測する。
   本観測では実際の進捗Board投稿後に占有が生じたが、投稿だけを唯一のトリガーとは断定しない。
2. `pane.list` を時刻・経過時間付きで実行し、同時に採取プログラムを約3秒間隔で実行する。
   採取プログラムの引数は PID と絶対ログパス。既存Debugging Toolsの x64 `dbgeng.lib` と
   Windows SDKヘッダを用いて x64 C++ としてビルドし、対応するDbgEng DLLと一緒に実行する。
3. `pane_backend_unresponsive` が出た標本について、同時刻のHTTP 101、main stack、
   `gwt.frontend.timing` を突合する。終了時刻から逆算したイベント区間には誤差がある。
4. 端末入力とGUI描画は別途、隔離インスタンスで確認する。PM/agentには未送信入力を置いて
   描画を観測し、他人の会話へ送信しない。GUIの通常操作、起動直後/restore完了後も別標本にする。
   WindowsとmacOSで同手順を行い、再現しない面はそのまま記録する。

| 面 | 現在の証拠 | 残件 |
| --- | --- | --- |
| PMウィンドウ | 既存PMコメントに障害記録。今回 backend timeoutを再現 | PM入力→描画の直接計測 |
| agentペイン | 今回9ペインを返す成功標本と、その後のbackend timeout | 各ペインのPTY受付とechoの分離 |
| GUI全体 | main の生成/待機stackと27–28秒dispatch占有 | 具体的クリックの応答時間 |
| 起動直後 | PMのmacOS報告に48セッション復元、9–12分の負荷 | 同手順での両OS制御実験 |

## 検証と保全範囲

採取プログラムのビルド、28回のattach/wait成功、JSON解析、時刻/ログとの照合を実施した。
最終ソースもMSVC `/W4 /WX /EHsc` でビルドし、PID 34952への追加smokeが成功した。
この成果は診断資料と採取用補助プログラムで、製品の動作変更ではない。
従前の `hook_health_test.rs` の未実行RED候補とマシン生成 `.codex/hooks.json` は本成果に含めない。
Rust test / clippy / headed dark-light / macOS / カバレッジ / 恒久計装の検証は未実施。

`workspace.ensure` はpurpose付きでも terminal bindingで拒否されたが、診断は続行した。
PM指示に従い `execution.reopen` は実行していない。Issueは未完了のまま保持する。

採取APIの契約は [Microsoftのnoninvasive debugging説明](https://learn.microsoft.com/en-us/windows-hardware/drivers/debugger/noninvasive-debugging--user-mode-)
を参照。今回使ったパッケージは `Microsoft.Debugging.Platform.DbgEng 20260319.1511.0`。

## 追加計装と最小修正（22:49 JST の検証）

上記は先行診断コミット `ebd29f32d` 時点の記録。後続変更では、次の計装と修正を追加した。

- WebSocket受信時刻をfrontendイベントへ保持し、queue wait、handler、受信からhandler完了までを分離する。
  既存の `elapsed_ms` はhandler時間の意味を保ち、合計30ms以上でWARNを出す。`window_count` は全window数。
  `queue_wait_ms` は受信からhandler開始までで、キュー滞留だけでなく認証・parse・配送前処理も含む。
- terminal fast pathは受信からPTY書き込み成功までを計測し、30ms以上でWARNを出す。
  `pty_writer_count` は同じwriter mapの要素数。入力本文は新規ログに渡さない。
- Work行のhook health集計を計測する。`work_count` はworktree未設定行も含む入力行総数で、
  project単独のhealth処理は含まない。既存のdispatchログと時刻を照合できる。
- hook binaryの比較対象がない場合、configのtracked判定を省く。
  戻り値はtracked/untrackedのどちらでもNoneなので、Git照会自体が不要だった。
  比較対象がある場合のcanonical fallback判定は維持する。

この修正は不要な同期Git照会を取り除くもの。27秒停止すべての解消を主張するものではない。
同期Work投影、cherry、error ledger走査などの残る占有時間は追加計測が必要である。

### 隔離したWindows Chromiumでの結果

checkoutの両binaryを再ビルドし、fresh HOMEとruntime junction、現在checkoutのsession seedを使って起動した。
HTTP 200、起動前後のhook convergenceでissuesゼロを確認した。
実Chromiumのheadedモードでdark/lightを切り替え、各条件3回、計18回のshell echoを測定した。
raw数値と選択した計測ログは [input evidence](issue-4172-input-evidence.json)、
採取手順は [measurement script](../../scripts/diagnostics/measure-input-4172.cjs) に保全した。

| shell数 | echo最小 / 中央値 / 最大 (ms) | dark / light 切替と確認 (ms) |
| --- | --- | --- |
| 1 | 121.1 / 122.5 / 150.5 | 75.4 / 83.5 |
| 8 | 109.5 / 121.1 / 128.1 | 114.3 / 102.4 |
| 24 | 118.9 / 119.5 / 139.2 | 181.4 / 147.7 |

この条件ではshell枚数によるecho遅延の増大は見られない。受信からPTY書き込みまでは118標本で0〜3ms。
内訳はshell宛115標本（0〜3ms）と、起動時agent宛の付随入力3標本（すべて0ms）である。
一方、GUIのUpdateWindowGeometryはqueue wait 211ms、handler 0msのWARNを残し、
入力受付が速い場合にもGUI待ちを分離できることを確認した。
dark/lightの画面を確認し、console error・page error・作成shellのcleanup errorはすべてゼロだった。

echoの測定対象はshellであり、実運用のPM/agent・復元されたWorkやerror ledgerの負荷は再現していない。
echo値にはshell実行と出力配送、animation frameの観測待ちを含む。テーマ値にはPlaywrightのclick待ちを含む。
同一条件の修正前測定はなく、性能改善率は算出していない。

### WindowsとmacOSの共通経路、過去修正との差

PMはBoard `fe2c1ad6-e29b-4f90-8dd2-e62260235503` と `c301b652-2f8d-4536-abfc-8deafce27450` で、
AC-1/2を診断結果から受け入れ、AC-5を「macOS実機採取ではなく共通コード経路の論証」と裁定した。
実機で同程度の停止が起きることまで検証した、とは扱わない。

現在のソースで `main.rs:9216` のBoardProjectionChangedは、`board.rs:677` からtitle syncへ同期的に進み、
`title_sync.rs:55` がcache-onlyではないWork投影を選ぶ。
`workspace_views.rs` のhook healthから `health.rs:439` のtracked判定へ進み、
`gwt-skills/src/settings_local.rs:596` がGitの `.output()` を同期実行する。
WorkTipSubjectsも、tip取得は別threadだが、結果反映時の投影再構築は同じイベントループ上である。
この経路にWindows限定の条件はない。`gwt-core/src/process.rs:356` のCommand生成と`:419`付近のOS分岐は、
Windowsのhandle/creation flagsを整えるもので、macOS側のGit実行を非同期化しない。
したがって、修正前に不要なGit照会でイベントループを待たせる構造は両OSに共通する。
待機handleの対象が未解決という、Windows stack採取の制約は引き続き残る。

Issue #2725の `545be4185` はcleanup候補算出のbranch列挙を削除し、repo hash計算のremote URL取得をconfig読取へ変えた。
今回のtracked config照会は、その後の `d97046bcf`（#3567関連）でhealth auditorへ追加された経路である。
以前の削除箇所がそのまま復活したわけではない。
今回の `configured?` による修正は、比較対象なしの場合の照会を単一レイヤで除去する。
Work投影全体を非同期化する契約変更は、この修正には含めない。

### 検証結果と未完了ゲート

| コマンド / 検証 | 結果 |
| --- | --- |
| `cargo test -p gwt --all-features --bin gwt handle_frontend_message` | 既存2件PASS、受信時刻の引継ぎを確認 |
| `cargo test -p gwt --all-features --bin gwt timing_warns` | 3件PASS（frontend、fast path、Work集計の30ms境界） |
| `cargo test -p gwt --all-features --bin gwt managed_hook_health` | 既存3件PASS（投影へのhealth付与、ambient状態除外、最新session選択） |
| `cargo test -p gwt --all-features --test hook_health_test` | 28PASS / 3FAIL。新規Git非実行回帰はPASS |
| `cargo build -p gwt --bin gwt --bin gwtd` | PASS |
| `cargo clippy --all-targets --all-features -- -D warnings` | workspace全体PASS |
| `cargo fmt --all -- --check` | PASS |
| `node --check scripts/diagnostics/measure-input-4172.cjs` | PASS |
| `npx --yes markdownlint-cli2 docs/diagnostics/issue-4172.md` | PASS |
| headed Chromium、dark/light、shell echo18回 | PASS。検査用GUI PID8916は検査後に停止 |
| `RUST_TEST_THREADS=1 cargo test -p gwt --all-features --bin gwt` | 停止を検知して中断、PASSではない |

hook healthの3失敗は、修正前の22:31 JSTの既存suite実行でも同じ名前で失敗している。

- `committed_install_path_in_tracked_codex_hooks_is_normalized_on_repair`
- `contaminated_tracked_codex_hooks_converge_back_to_canonical`
- `repairing_another_worktree_never_writes_a_worktree_local_build_path`

修正前は26PASS/4FAILで、上記3件に加え、自動生成されたWindows用 `.codex/hooks.json` に対する
portable assertionも失敗していた。tracked版へ戻して再ビルドした修正後は、この追加1件が解消した。
したがって上記3件は今回のGit guard導入による新規失敗ではない。比較対象のない新規回帰は、
修正前にGit Trace2の `ls-files` を検出してRED、修正後は同じfixtureでGREENだった。

main suiteは1498件の実行を開始したが、`explicit_pm_open_bypasses_the_crash_backoff_floor` の途中で停止した。
PID13368の出力は23:03:01〜23:03:45 JSTで95126bytesのまま、CPUは58.640625→58.6875秒。
その直接子には `cmd /d /s /c "exit /b 0"` が9個残っていた。
実行前から `RUST_TEST_THREADS=1` を設定しており、並列度制限だけで解消したとは扱わない。
現在checkoutの当該test process treeだけを停止し、leaseを解放してPMへ報告した。
このsuiteと全体coverageは検証完了ではなく、canonical verification / User Verification / PR gateも未完了である。

Launch mode: interactive。Agent Visual Check: pass（上記の限定条件）。User Verification Result: pending。
Overall: FAIL（既存fixture失敗、main suite中断、受け入れ残件）。Ready PRを作成しない。

PM Board `3231fce0-0970-427c-afd6-928c49228684` により、AC-4の次の調査対象は
実運用Work / Board / error ledgerの蓄積量へ変更された。
shell枚数については「今回のfresh HOME条件では相関を認めない」と記録し、あらゆる実運用条件で無関係と断定しない。
実運用状態のコピーを冷起動すると既存worktreeのhook self-heal等が走るため、HOME隔離だけで無変更は保証できない。
追加比較は、セッション復元を避ける安全な採取方法の確定から引き継ぐ。

### 実運用データ量の読み取り集計（23:06 JST）

実USERPROFILEの `.gwt/projects/99a8660247f5bc49` には以下のデータが存在した。
本文・秘密情報は採取資料に含めず、原本の変更・コピー・GUI再起動を行っていない。

| 保存先 | 件数 | bytes |
| --- | --- | ---: |
| `project-state/works.json` | 保存Work 254件 | 3,383,335 |
| `project-state/current.json` | — | 15,219 |
| `project-state/journal.jsonl` | 257行 | 132,045 |
| `coordination/board.latest.json` | hot 500件 / total 1,680件 | 698,473 |
| `coordination/events.manifest.json` | 1ファイル | 528 |
| `coordination/events/*` | 1ファイル | 1,992,612 |
| `.gwt/logs/errors/errors.*.jsonl`（HOME共通） | 10ファイル / 1,122行 | 644,912 |

`ffa30e30ecb522d5` にはlogs/runtimeのみがあり、project-state/coordinationはない。
実運用ログのhashとWork保存先のhashを混同しない。保存Work総数254は、grouping後の
hook health対象行数 `work_count` と同じとは限らない。

コード上、`error_ledger.rs:198` は保持日数外を含む全ledgerファイルを読み、JSON parse後に日時で絞る。
`health.rs:282` はWork別healthごとにこの読取を呼ぶため、同じledgerの読取・parseが繰り返される。
ただし、このデータ量だけから27秒の原因だとは結論しない。各呼出しの所要時間は未採取である。

再現用のコピー候補は上記project-state/Board/ledgerのみで、sessions、workspace、PM registration、
execution/trusted情報、runtime状態、lock/transaction、旧work-eventsは移送しない。
PM自動起動は隔離先 `project-state/pm.json` の `settings.auto_start=false` で抑止できる。
startup後にBoard latestを変更するとwatcherが投影を起動するが、handlerにはmilestone transactionがあり、
現在checkoutのrepo-local Work eventへ書く可能性が残る。
したがって、実行前にその書込先も隔離できることを確認する必要がある。この後注入案は未実行である。

### health 単体の実 ledger 量比較（23:36–23:38 JST）

GUI を起動せず、通常ビルドの公開関数 `read_managed_hook_health` を同一の空 fixture に対して
反復した。実 ledger のコピーを使うと、評価回数と ledger 量の両方に応じて時間が増えた。
全投影の再現ではないが、Work ごとの ledger 全走査が秒単位の負荷になることを実測した。

| ledger 条件 | 非空行数 / bytes | 1評価 | 32評価 | 254評価 |
| --- | --- | ---: | ---: | ---: |
| 空 | 0 / 0 | 0.8ms | 22.2ms | 175.4ms |
| 各ファイルの先頭から一行おきに抽出 | 576 / 325,439 | 13.1ms | 418.3ms | 3,158.6ms |
| 実 ledger コピー | 1,145 / 657,882 | 23.6ms | 758.4ms | 5,861.1ms |

各値はウォームアップ1回後の3回の中央値。通常の dev（最適化なし）rlib にリンクした
[診断プログラム](../../scripts/diagnostics/measure-work-health-4172.rs) を使用した。
[証拠JSON](issue-4172-health-evidence.json) に全27標本、入力ファイルのhash・サイズ、
ソースcommit、compiler versionを保全した。ledger本文やhealthのメッセージは出力しない。
全コピーは10ファイルで、物理行数1,146と非空行数1,145を区別する。

比較する関数呼出しの数は1/32/254だが、**254個の異なるWorkを投影した測定ではない**。
空fixtureを繰り返すので、実worktreeの設定・discussions・session読取、grouping、
Board読み取り、Git起動、描画は含まない。実ledger内のproject pathはfixtureと一致しないが、
path照合の前に全行がパースされる。結果のissue数は全条件ゼロだった。
実運用releaseの約28秒にこのdebug測定値をそのまま足し引きしてはならない。

量依存の経路は `workspace_views.rs:403` のWork反復から、同`:380`、`health.rs:139/:282`、
`error_ledger.rs:198` の全ファイル読取・`:223`以降の全行パースへ続く。
日時とprojectによるfilterはパース後なので、表示対象外のエラーも評価ごとに読み直す。
空ledgerとの差は254評価で約5.69秒、半量との差は約2.70秒だった。

再実行には、先に正規verification leaseを取得し、このcheckoutの通常Cargo buildで作った
gwt rlibへ診断プログラムを `rustc --edition=2021` でリンクする。
`--extern gwt=<rlib>` と `-L dependency=target/debug/deps` を渡す。
Windowsでは `windows_x86_64_msvc-0.53.1/lib` のnative library search pathも必要だった。
その指定前のlinkは `LNK1181: windows.0.53.0.lib` で失敗し、指定後は成功した。
test-support有効のrlibは使用しない。その構成では専用home overrideなしのledger読取が空になる。

子processごとにfresh HOME/USERPROFILEを同じディレクトリへ設定し、
`.issue-4172-measurement-fixture` markerと配下の空fixtureを用意する。
ambientなGWT/Git環境変数を子環境から除去し、引数にfixture、`1,32,254`、`3`を渡す。
空条件はledgerなし、全量条件は原本から読み取りコピーした `.gwt/logs/errors`、
半量条件はコピーの各ファイルから一行おきに抽出したデータを使う。
この採取で原本の変更、実worktreeへの書込、GUI起動は行っていない。

次の修正候補は、一度の投影でledgerを一度読み、各Workへ同じsnapshotを渡す方法である。
永続cacheの失効管理を追加せず、現状のWork別filterを保てる。ただしこれは未実装・未検証であり、
既存SPECのタスク追加と修正範囲の裁定を要する。全投影時間・子Git数・Board量の比較は依然残る。
AC-4/6とOverallを完了にせず、既存PR #4189はDraftのままとする。
