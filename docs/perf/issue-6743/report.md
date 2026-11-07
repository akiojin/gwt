# Issue #6743: Windows idle CPU

本資料は各測定時点の証跡。R5 の直接 shipping Dark 600 秒は GUI / daemon の
CPU 閾値を満たした。下記の環境拒否は当時の結果として保持する。
Issue #6735 の回復機能は PR #6762 で配送され、現在の作業ブランチにも取り込み済み。
全 AC の完了は、最終 HEAD の fresh canonical 検証と develop 配送の確認で判定する。

分類は gwt runtime の性能機能。開始時の fetch 後、HEAD と
origin/develop は `bcf777ed432df9fd96dc5dd9c13d01ec859d7ea2` で一致し、
未配信ソースと linked/current PR はなかった。既存 SPEC #5390 の
spec/plan/tasks に今回の範囲を登録した。

## 計測条件

- Windows、通常 Launch で作成した異なる 5 Session / 5 PTY。
- 各 PTY は 600 行の履歴を生成後に静止。計測中はユーザー入力と
  terminal output がなく、認証付き pane.read は各窓で継続する。
- 同じ private project に属する GUI / Monitor OFF daemon を同時計測。
  PM auto-start は無効。background index と周期処理は通常どおり。
- 元 Session 情報の非公開コピー 7,428 files / 37,508,132 bytes を共通入力にする。
  fixture SHA256 は
  `eb11ceef7c828dc6a590fe4adce02bfbe3dd5927002115ff5548fafeaead83dc`。
  コピー本体や認証情報は配送資料に含めない。
- CPU は kernel + user の process CPU 秒を実測 wall 秒で割った値。
  1 logical core = 100% とし、GUI <5%、daemon <1% を判定する。
  machine-normalized 値と logical processor 数も各 JSON に残す。
- 配布性能は Release、動作確認と関数名解決は Debug で分離する。
  Debug の短い CPU 値を Release の受け入れには使わない。

fresh HOME の startup intake は、GUI 開始後の v11 index commit、primary
project の snapshot、正の source 数、source-list fingerprint で確認する。
現行 Launch は新 worktree の再 intake を要求しないため、5 窓モードでは
その取り込みを開始条件にしない。新規 worktree の未処理状態は診断に残す。
旧 12 窓モードの全 worktree 条件は変更していない。

## 初回処理を含んだ旧区間の実測

以下の旧区間には履歴 Work の初回 hook audit が混入していた。
起動後の負荷として保存し、steady-idle の性能受け入れには使わない。
その後の測定は、保存済み Work ID 全件と現在の 5 Session の health を
含む full `active_work_projection` の到着後に開始する。
membership-only patch は完了証拠に使わない。

| Process | CPU 秒 | 実測 wall 秒 | logical-core CPU | 基準 |
| --- | ---: | ---: | ---: | ---: |
| GUI | 180.640625 | 600.0839037000001 | 30.102561% | <5% |
| Monitor OFF daemon | 47.468750 | 600.0754111 | 7.910464% | <1% |

両方の性能基準を超過し、保存結果に対する閾値 assertion も RED / exit 1。
headed dark/light の機能 assertion は各 1 PASS。出力ゼロ、console/page
error ゼロ、全 5 窓の前後 input probe とプロセス終了を確認した。
light の 30 秒は追加観測で、10 分 AC の代わりにはしない。

原測定は [GUI](baseline-dark-idle-cpu.json)、
[daemon](baseline-dark-daemon-cpu.json)、
[起動・binary hashes・終了](baseline-launch.json) に保存する。

## 関数 profile

WPR は `0xc5585011` / “Failed to enable the policy to profile system performance”
で開始できなかった。権限設定は変更していない。

owned checkout の PID / 開始時刻 / executable と各 thread の owner PID を
検証し、Running-thread snapshot、短い suspend、GetThreadContext / StackWalk64、
resume で採取した。resume 失敗は明示的に失敗とする。
Debug の初期準備後、30.6321373 秒で 437 stacks、context 失敗 0。
この profile にも初回 hook audit の活動が含まれる。
採取区間は 10 分 CPU 計測から除外した。

| 上位 leaf 関数 | 標本数 |
| --- | ---: |
| NtCreateFile | 91 |
| ZwCreateUserProcess | 63 |
| ZwQuerySystemInformation | 40 |
| NtClose | 34 |
| NtDeviceIoControlFile | 19 |

| 最初に現れる gwt 関数 | 標本数 |
| --- | ---: |
| pm_registry::pm_worktree_path_for_repo_path | 194 |
| `cli::hook::health::expected_hook_bin_for_config_path::<closure>` | 39 |
| index_worker::poll_registered_project_index_sources | 35 |
| cli::hook::health::audit_managed_git_hooks | 29 |
| cli::tray::lifetime::refresh | 26 |

標本数は CPU 秒や占有率ではない。Running-state の観測 race、sampling
overhead、unwind の制限がある。usage collection は 219 stacks に含まれ、
PM path 解決への反復呼び出しを source でも確認した。process 作成の
63 標本は hook binary 期待値 35 / Git hooks audit 28 であり、PM path
解決の subprocess 作成とは扱わない。

[採取 JSON の LF 正規化コピー](baseline-profile.json) の SHA256 は `ad24df0a0220f1c614adfe131055da7d3ff19fe53a8f1fc22594f8a09952a453`。採取元の原 bytes の SHA256 は
`a27cd4f13cefb7768a609caba364936234088f9f7ea417996b4a51d76e136fd5`。
[集計](baseline-profile-summary.json) と同じ上位 5 関数を Issue に記録した。

## 最小修正

usage collection は通常の memberless worktree でも期待 PM path を解決してから
PM-shape predicate で棄却していた。cheap predicate を先に評価し、
canonical PM / registered PM / member identity / NoData / ambiguity の結果を保つ。

Monitor runtime reconciliation は captured entry が空でも全 Session / OS census を
5 秒ごとに実行していた。空の場合だけ dead-host counts の publication を残して
census を省略する。非空の場合の fresh census、stop receipt、checkpoint、
tree cleanup、権限確認と polling 周期は既存のまま。

fixture の静的再評価モデルでは、元の PM path 解決は約 2,575 回 / poll。
うち通常 worktree は 2,224 回。残りの元 PM path も、この private HOME の
canonical PM ではないため省略できる。コピーは元 path を保っており、
installed fleet の canonical PM 負荷まで測ったものではない。
共有 conversation 群の二乗規模は潜在値で、この fixture の主因とはしない。

最新ソースで monitor 8 件、inventory 26 件、usage identity 3 件、計 37 件が PASS。
空 host の 5 秒 lock deadline を保持する修正を含め、独立 review の blocking は 0。
この限定計測を、親 SPEC の実 fleet / installed rollout / 長時間 memory 基準へ
転用しない。

## 残る負荷の調査

最初の修正 R1 の旧 600 秒区間は GUI 104.578125 CPU 秒 /
600.0748991 秒 = 17.427512%、daemon 34.703125 /
600.0702069 秒 = 5.783177% で、ともに RED。GUI の区間内に
895 Work 行 / 138,206 ms の初回 audit 完了が記録された。
旧区間は前記と同様に cold 処理を含み、steady 比較とは区別する。

R1 の追加 daemon profile では 137 標本のうち 92 が
`host_process_start_epoch_secs`。呼び出し元は frontend dispatch 73、
owner request 14、Monitor counts 5。Monitor の周期変更ではなく、
要求ごとの Windows process 情報収集が対象となる。
[daemon 原記録](r1-daemon-profile.json) と
[集計・binary hash](r1-daemon-profile-summary.json) を保存した。
GUI index の別集計は index poll を含む stack 全件を母数とし、
44 標本中 40 がパスの canonicalize を含む。
[集計と代表 stack](r1-index-profile-summary.json) の全 profile は 291 標本。
inventory 16,206 候補のうち 14,177 は lexical builtin-denied だった。
reparse / junction / symlink の意味を保持する範囲に修正を限定する。

R2 は Windows の開始時刻取得を `GetProcessTimes` と同じ owned handle の
稼働確認へ変更する。1,000 回の開始 identity 取得を加えた既存テストは、
変更前 17.0625 CPU 秒で 0.5 秒の予算を超過した。
非 Windows と test clock、秒単位の identity、失敗時の未知扱いを保持する。

index batch は実在する完全一致名と非 reparse 属性を確認した通常の
除外パスだけ canonicalize を省略する。metadata は 1 batch 内で共有し、
allowed / missing / unknown / reparse は既存 scalar 判定に戻る。
通常除外 2 ファイルの RED は canonicalize 2 回、期待値は 0 回。
Windows の case spelling と junction の両方向・次 batch の変更を検証する。

## full 投影完了後の同条件比較

両 run とも full Work IDs 1,751/1,751、missing 0、active rows 896、
現在の 5/5 Session の root と監査結果を照合してから計測した。
監査結果の `needs_attention` は監査完了を示すもので、健康性の成功とは扱わない。
Debug / Release の追加 build と並行した直接診断であり、最終 canonical
verification の代わりにはしない。

| Generation | Process | CPU 秒 | wall 秒 | logical-core CPU | 結果 |
| --- | --- | ---: | ---: | ---: | --- |
| pristine base | GUI | 152.453125 | 600.0932814 | 25.404904% | RED (<5%) |
| pristine base | Monitor OFF daemon | 55.000000 | 600.0850379999999 | 9.165368% | RED (<1%) |
| R1 | GUI | 82.500000 | 600.0694548 | 13.748409% | RED (<5%) |
| R1 | Monitor OFF daemon | 40.984375 | 600.0776251 | 6.829846% | RED (<1%) |

base の [GUI](steady-baseline-idle-cpu.json)、
[daemon](steady-baseline-daemon-cpu.json)、
[準備完了](steady-baseline-work-projection-readiness.json) と、
R1 の [GUI](steady-r1-idle-cpu.json)、[daemon](steady-r1-daemon-cpu.json)、
[準備完了](steady-r1-work-projection-readiness.json) を保存した。
比較 binary は保存済み immutable pair。
[R1 source generation](r1-source-generation.json) と binary SHA256 で特定し、
harness capture 時点の作業ツリー hash を、その binary の build source と混同しない。

## R2 / R3 と残件

| Generation | Process | CPU 秒 | wall 秒 | logical-core CPU | 結果 |
| --- | --- | ---: | ---: | ---: | --- |
| R2 | GUI | 57.765625 | 600.0622797 | 9.626605% | RED (<5%) |
| R2 | Monitor OFF daemon | 9.484375 | 600.0822934 | 1.580512% | RED (<1%) |
| R3 | GUI | 41.093750 | 600.4208998 | 6.844157% | RED (<5%) |
| R3 | Monitor OFF daemon | 9.125000 | 600.08215 | 1.520625% | RED (<1%) |

R2 は [GUI](steady-r2-idle-cpu.json)、[daemon](steady-r2-daemon-cpu.json)、
[準備完了](steady-r2-work-projection-readiness.json)、
[source generation](r2-source-generation.json)。
R3 は [GUI](steady-r3-idle-cpu.json)、[daemon](steady-r3-daemon-cpu.json)、
[準備完了](steady-r3-work-projection-readiness.json)、
[source generation](r3-source-generation.json)。実際の分母と値は JSON を正本とする。

R3 は Windows `--no-tray` parent liveness の 500 ms ごとの全 process
snapshot を native helper に統一した。確定 dead / 開始 identity 不一致のみ
終了し、観測不明は保持。capture、Unix、周期、browser reconnect grace は保持する。
既存 parent test の RED は snapshot counter 6 vs 2、GREEN は追加 0、
既存 lifetime 2 件が PASS。独立 review の blocking は 0。

R4 の追加は singleton conversation の未使用 identity 確認を省く 1 行。
既存 common-clock fixture の RED は呼出し 4 vs 0、GREEN は 0。
既存 collect_sessions_ 3 件と strict gwt all-target/all-feature Clippy が PASS。
multi-owner identity / selection と出力 project_path の取得は保持する。
R4 の shipping 600 秒実測も両方 RED。

| Process | CPU 秒 | wall 秒 | CPU fraction | logical-core CPU | 結果 |
| --- | ---: | ---: | ---: | ---: | --- |
| GUI | 33.609375 | 600.0886088000001 | 0.056007353759320345 | 5.600735% | RED (<5%) |
| Monitor OFF daemon | 8.703125 | 600.0890807999999 | 0.014503055093749677 | 1.450306% | RED (<1%) |

[GUI](steady-r4-idle-cpu.json)、[daemon](steady-r4-daemon-cpu.json)、
[source generation](r4-source-generation.json) と
[準備完了](steady-r4-work-projection-readiness.json) を保存した。
この run の full 投影は Work 1,752/1,752、missing 0、active rows 897、
現在の Session 5/5 を照合済み。旧 run の 1,751 件とは区別する。

Light の cached full replay は次の prepare 完了を証明しなかったため、
5-pane mode は Dark を通常 cleanup し、Light を新しい 5 Session / root で
通常 Launch する。既存 full-event / exact root+Session / non-null health 条件は、
新規 group の cache-only patch を拒否できる。旧 12-pane reuse は保持する。
修正版の Debug headed dark/light 各 1 PASS、全 5 窓の入力、エラー 0、
画面表示と owned cleanup を確認した。

R2 の canonical pre-pr は [記録要約](r2-verification-summary.json) の
12 commands PASS / 1 environment incompatible。実 Chromium は開始前に拒否された。
`PID 46296 has unregistered ancestor gwt PID 54020` が理由で、既存修復 owner は
Issue #6735。共有 GUI / Job registry は変更していない。
直接 headed の PASS は canonical PASS や uncapped 証明の代わりにしない。

stripped shipping Release の native symbol は大きな displacement を伴う
nearest public symbol しか得られず、Rust 関数に帰属できない。kernel leaf の
観測だけを保全し、line-tables-only / unstripped の既存 bench profile を
独立 target に構築して残因を調べる。この診断 artifact は shipping CPU AC に使わない。

User Verification Result: n/a (autonomous)

## R4 optimized profile（関数帰属専用）

shipping run `b01cf9b2dee04eb5ac2d908e3754edfb` とは別に、diagnostic run
`c7c07b025c264aabaf5eb07138f2ff96` を既存 bench profile の
`debug=line-tables-only` / `strip=none` で採取した。出力先 directory と
harness の profile 表示は `release` だが、shipping CPU AC には使わない。
両 source generation に記録された 6 source-file hashes と commit association
`8d2087d43d989904ae94f38cdd4395c3c1e7c69f` は一致する。binary は異なる。

[集計・provenance・代表 stack](r4-optimized-profile-summary.json) に raw の
local path / SHA256、shipping の保存先 / SHA256、各 binary hash と diagnostic
source generation を記録した。巨大な raw profile 全体は配送資料へ複製していない。
GUI は 120.3281053 秒 / 225 samples、daemon は 120.2466389 秒 / 52 samples、
context failure は各 0。標本数は CPU fraction、CPU 秒、CPU% ではない。

以下は関数名の短縮表記。leaf は各 sample の先頭 frame を 1 回だけ数える。
上位 5 行の cutoff と同数の関数はすべて載せる。

| GUI leaf | samples / 225 |
| --- | ---: |
| NtCreateFile | 46 |
| ZwQuerySystemInformation | 32 |
| NtDeviceIoControlFile | 20 |
| NtQueryInformationFile | 15 |
| NtClose | 14 |
| _lread | 14 |

| daemon leaf | samples / 52 |
| --- | ---: |
| ZwCreateUserProcess | 17 |
| NtCreateFile | 10 |
| NtClose | 4 |
| NtWaitForAlertByThreadId | 4 |
| NtReadFile | 3 |

inclusive は同じ関数を 1 sample 内で最大 1 回数える。入れ子の関数が重複するため、
行を合計して占有率にしない。ここでの product 関数は symbol が `gwt` で始まるもの。

| GUI inclusive product | samples / 225 |
| --- | ---: |
| agent_capacity::RuntimeProbe::refresh_projects_with_scan | 76 |
| usage_poller::Poller::poll_once | 46 |
| runtime_health_poller::run | 32 |
| usage_poller::collect_sessions_with_clock::closure$1 | 28 |
| session_inventory::capture_process_resources_with_system | 22 |

| daemon inclusive product | samples / 52 |
| --- | ---: |
| process_console::spawn::spawn_logged_inner | 24 |
| process_console::spawn::block_on_spawn::closure$0 | 24 |
| process::resolved_tokio_command | 6 |
| process::windows_resolver::resolve_process_plan_for_platform | 6 |
| process::windows_resolver::resolve_windows_candidate | 6 |
| process::windows_resolver::resolve_windows_target | 6 |
| session::inspect_session_path | 6 |

optimized inlining により product 名が generic retain / closure environment の
型名内に入ることがある。GUI の gwt-prefix frame 不在 35 samples のうち 29 は
index thread のこの表現だった。frame 文字列内の owner 名も含める別集計では
capacity 76、usage 46、index drain 45、runtime-health 32。
index filter は 29 samples、そのうち canonicalize を含むものが 19。

R4 の singleton guard 後にも、usage poll 46 samples 中 43 に canonicalize があり、
confirmed-identity closure 28 samples はすべて canonicalize を含む。
R4 source では残る multi-owner group の判定を各 reference で再評価している。
これは初期 fixture の静的評価とは別の R4 観測であり、sample 数から削減 CPU を
見積もったものではない。

daemon の spawn helper は inclusive 24、process-creation leaf は 17。
別の caller stack 2 samples には `github_remote_owner_and_repo` から spawn helper
の完了を待つ経路があり、R4 source の command は `git remote get-url origin`。
spawn thread 自体の sample に command arguments や caller との対応記録はないため、
17 samples 全件をこの Git command に帰属させない。stripped shipping の nearest
public symbol と、この診断の product 件数も比較しない。

## R5 の残因修正と検証

usage は同じ collection 内の conversation group の判定だけを共有する。
既存 shared-owner fixture の identity 呼出しは RED 14 vs 5 から GREEN 5 へ。
既存 collect_sessions_ 3 件が PASS し、次 collection では新しい foreign PM を
反映して NoData に戻る。出力順・PM/member 選択・singleton・出力 path は保持する。

daemon の native origin shortcut は、fresh な壊れた HEAD/config の Git 診断を
迂回するという独立レビュー指摘を受けて完全に撤回した。新しい Git parser や
正の validation cache と比較し、既存 GUI の OFF/後処理なし定期 scan 省略を
daemon の自動 timer/capacity 要求に適用する小さい修正を選んだ。
現在の runtime と現在/最新 disk の driver・cleanup 状態が空の場合だけ省略する。
読取失敗、ON、active/cleanup、明示要求、retry は従来の処理を維持する。
Git resolver と IssueMonitorState の変更はない。

実 worker の RED は不要な自動 scan 2 回 vs 期待 0 回、GREEN は 0 回で、
同じ test の後段の ScanNow は実行された。最新 binary の関連 worker 32 件は
ON の周期処理、容量更新、retry、explicit、fresh disk の境界を含めて全 PASS。
両 source diff の独立レビューは blocking 0。workspace all-target/all-feature
strict Clippy、整形、test hygiene の関連 3 件も PASS。

Debug の隔離 headed 実 Chromium は dark/light 各 1 PASS、各 theme は異なる
5 Session、各 PTY は 600 行、console/page error 0、所有 GUI/daemon 終了を確認した。
[証跡要約](r5-functional-summary.json) に source binary hash と原 artifact の hash を残す。
この functional run は persisted Work frontier を必須にせず startup を含むため、
短い CPU 値や health の needs_attention を steady 性能・健康性の成功には使わない。
R5 の直接 shipping Dark 600 秒は両 strict CPU 閾値に PASS。

| Process | CPU 秒 | 実測 wall 秒 | CPU fraction | logical-core CPU | 結果 |
| --- | ---: | ---: | ---: | ---: | --- |
| GUI | 28.578125 | 600.0889017 | 0.04762315203470793 | 4.762315% | PASS (<5%) |
| Monitor OFF daemon | 3.03125 | 600.0822326 | 0.005051391018304914 | 0.505139% | PASS (<1%) |

[GUI](steady-r5-idle-cpu.json)、[daemon](steady-r5-daemon-cpu.json)、
[full 投影の準備完了](steady-r5-work-projection-readiness.json)、
[source generation](r5-source-generation.json)、
[最終集計と全 provenance](r5-performance-summary.json) を保存した。
Dark は Work 1,752/1,752、missing 0、active rows 897、現在の Session 5/5 の
root と non-null health を照合後に計測した。needs_attention は監査完了の結果であり、
健康性の PASS とはしない。

同じ GUI 上で別の 5 Session / root を通常 Launch した Light は追加 30 秒。
GUI 1.0625 / 30.0614369 秒 = 3.534429%、daemon 0.125 / 30.0516355 秒 =
0.415951% で短時間の閾値は満たしたが、600 秒 AC の代わりには使わない。
Light の full 投影は Work 1,757/1,757、missing 0、active rows 902、Session 5/5。

各 theme の console/page error は 0、前後各 5 件の入力 probe を照合した。
CPU 計測中の全 5 窓は raw bytes/chunks 0、入力 0。
Dark IPC は計測内 275/275 成功。Light は末尾で 14 attempts / 13 successes だったが、
残る read は CPU 区間終了後の 21:32:39.613Z に成功完了しており、失敗記録はない。
最終 .last-run は passed、owned cleanup は GUI 33424 / daemon 34764 とも exited=true、
記録時刻は 2026-11-04T21:32:52.7206239Z。

fixture clone は従来と同じ `8d2087d43d989904ae94f38cdd4395c3c1e7c69f`。
R5 binary はその base 上の working tree を compile したもので、fixture SHA を
R5 の source commit と扱わない。7 source-file hashes は source generation と
launch で照合し、Release binary hashes はそれらと CPU snapshot で照合した。
保存は UTF-8 の LF 正規化だけとし、3 計測/準備 JSON は raw と同じ bytes。
source generation は CRLF→LF により SHA256 が変わるため、raw / 保存後の両 SHA を
最終集計に記載した。

この直接測定は canonical uncapped の証明ではない。全 AC / PR / develop 配送は未完了。

## pushed source の最終 canonical 結果

source と証跡は `5811e37b989f28908586832af678fe62d432fb55` に commit/push した。
この HEAD の derived pre-pr 20 commands を改めて verify.run した記録は
`vrr-ea6904466bca4308bb92d13b222fcdeb`。17 commands PASS、製品テスト失敗 0、
environment incompatible 1、外部 signal による中断 0。
[記録要約](r5-verification-summary.json) に精算結果・各 command・原記録 hash を保存した。
private な execution authority は含めていない。

headed 開始前に `PID 13100 has unregistered ancestor gwt PID 54020` で拒否された。
raw_exit_code は null、合成 exit 125、fresh Chromium dark/light はともに 0 実行。
CLI の自動 artifact 復元は PASS。fail-fast で後続 3 planned commands を省略し、
別の自動復元を含めた結果数は 18。plan_covered=true は全 command の成功を意味せず、
all_passed=false である。

この run の named regressions は monitor 8、inventory 26、bin fixture/usage 5、
process 17、lifetime 2、OFF worker 1、fresh disk 1、ON worker 1、index counter 1、
index integration 8、hygiene 36 PASS（既存 ignored 1）。strict workspace Clippy/fmt と
helper contract も PASS。受け入れの CPU 直接測定と、canonical の環境拒否を区別する。

依存 #6735 の clean source `892ef9cfb` には同一 verify.run を Windows worker へ
転送する回復実装があるが、他 checkout の driver を指定する公開経路はない。
embedded driver source HEAD と対象 HEAD の完全一致が必要なため、この binary を
5811 の代用にはしない。独立した source 調査結果を PM へ引き継いだ。
共有 GUI/registry/authority/gate の手動改変は行っていない。

この後の記録保全は Work/文書だけで、runtime source の変更ではない。
Issue #6743 と親 #5390 は OPEN、AC は unchecked、PR/CI/merge は未実施。
coverage は未測定（PR 未作成）。outer shell 除外後の gwt-core+gwt 90% と他 domain 80% の
必須 CI gate を未達と判定したものではない。

## 2026-11-06 の配送再開

サチが fetch 後の `origin/develop` と比較した時点で、HEAD
`dbbff3c1054d48a2c28fe63df7a08d52b2a232bb` の固有 6 commit は未配信だった。
既存の runtime 修正を追加実装せず維持し、同 checkout の正規 worker 回復と
元 caller の `hook.doctor` broker を使って検証・配送する。
前回の canonical 記録 `vrr-78632491ca37496bbfd19462f58a81ab` は Debug の
headed dark/light 各 1 PASS と関連テストを含む 17 commands PASS の後、
Release build 中の記録で止まっており、全 matrix PASS としては扱わない。

独立した読み取り専用差分レビューでは blocking 指摘はなかった。
fresh な最終結果、PR、必須 CI の scoped coverage、merge と base 包含の証明は
Issue #6743 と配送 PR に記録する。branch 上の source や旧測定値だけを根拠に
AC を完了したり Issue を閉じたりしない。

User Verification Result: n/a (autonomous)

## 2026-11-07 の canonical 計測区間の修正

サラ・カトウ。最終候補 `f5d4e3e1e4f55f1417205714cca7e9946169bfc7` の canonical
`vrr-e88a0e143f6247bfaab3720f18c6b11e` は Debug headed dark/light 各 1 PASS、
関連 Rust / index / hygiene / helper と Release build まで成功した。
Release の Dark は Work 1,792 件・missing 0・現在 5/5 Session の監査完了後に測った。

| Process | CPU 秒 | wall 秒 | logical-core CPU | 600 秒の判定 |
| --- | ---: | ---: | ---: | --- |
| GUI | 26.5625 | 600.0602237 | 4.426639% | PASS (<5%) |
| Monitor OFF daemon | 4.6875 | 600.0724527 | 0.781156% | PASS (<1%) |

一方、Light は harness が区間を 30 秒へ短縮したまま同じ閾値を適用していた。
GUI は 1.515625 / 30.0562746 秒 = 5.042624%、daemon は 0.375 / 30.0535454 秒 =
1.247773% で daemon の budget assertion が失敗した。両テーマの errors は空だった。
この run 全体を PASS と扱わず、原 artifact `issue-5658-after-5fa99ba8a7da42738f07f04b982ae2e0`
と canonical 記録を保全する。30 秒の値は 600 秒 AC の未達を示すものではない。

根因は CPU 判定の要求時間と Light の実測時間の不一致。親 SPEC #5390 の plan/tasks を
更新し、いずれかの CPU budget を要求した場合は両テーマで指定された全区間を測る。
budget を要求しない Light の短い診断は保持する。変更は既存 E2E の区間選択と本報告だけで、
runtime、閾値、監査・IPC・入力・エラー・cleanup の assertion は維持する。
既存の canonical failure を RED 証跡とし、修正後の fresh full matrix で GREEN を確認する。
配送はその後の Ready PR / 必須 CI / auto-merge / develop 包含確認で確定する。

## 2026-11-07 canonical CI integration repair

`d7c4d1a14` の fresh canonical は22/22成功し、Release両テーマ各600秒の
GUI/daemon基準も満たした。ただしPR #6836のLinux coverageはテスト失敗で
未集計となった。閾値未達とは判定しない。

- fleet fixtureの`coreutils: unknown program codex` / EOFは、既にdevelopへ
  配信された`d6009d7ee`のRust handshake fixtureを取り込み、重複修正を避ける。
- `transport_direct_dispatch_inventory_stays_explicit`は、今回のusage counterが
  production sourceより前にある最初の`#[cfg(test)]`となり、dispatchを数える
  既存inventoryを短縮した。既存テストで0対期待1のREDを再現した。
- 同じcounter blockをproduction関数の後へ移し、inventoryと既存usage3件の
  計4件でGREENを確認した。runtimeの処理・counterのscope・assertionは維持する。

上記の旧記録と数値は旧HEADの証拠として保持する。取り込み後のHEADについて
新しい全canonical matrix・実headed両テーマ・Release各600秒を完走し、必須CIと
fetched develop包含を確認するまで配信完了とは扱わない。
