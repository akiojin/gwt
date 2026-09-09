# Issue #4172: Windows 入力停止の診断（2026-09-09）

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
