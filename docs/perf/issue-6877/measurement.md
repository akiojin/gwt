# Issue #6877: automatic scan retry reads

## Scope and baseline

サチコ / Codex。分類は gwt runtime の性能修正。評価した base は
`740d5960d25051d71d4f97011b10708174ecf911`。PR #6889 の source は既に
base に含まれ、継承 branch に残った未配送 commit は Work 記録だけだった。

Goal は自動 scan 再試行が既存 TTL を迂回する原因の除去。
UI、hook health、prefs-lock の設計、deadline、claim/permission authority、
live prelaunch confirmation は変更範囲外。

Acceptance Surface: non-user-facing (この slice は daemon の remote read
policy と subprocess 再起動回数を検証する。描画、GUI 操作、通知 payload、
frontend schema の変更はない)。ブラウザ検証はこの slice の対象外。

User Verification Result: n/a (autonomous)

Agent Visual Check: n/a (no UI surface)

## GUI の stale refresh 迂回（2026-11-10、次スライス）

アヤ・ミウラ / Codex。開始時の fetch 後 HEAD と origin/develop はともに
`4e39a85b426128c056342eed95767fb17918d8cd`、branch-only commit は0。
PR #6976 までの source は配送済みで、既存修正を再実装しない。
最新 SPEC #4093 の spec / plan / tasks を読み、FR-6877-4 と
T-6877-R10〜R12 に今回の境界を記録した。

### 残存する根因と最小 Plan

GUI の index worker と knowledge bridge は stale-only refresh を使うが、
その wrapper は明示 refresh と同じ live-only 経路を呼ぶ。
途中で失敗すると full-refresh marker が stale のままなので、
Monitor が取得済みの general REST page も次の GUI retry が再取得する。

TTL 延長は迂回を直さず、新しい cache / single-flight 層を必要とする
証拠もない。既存900秒 page reuse に stale-only wrapper を接続する。
full-refresh 失敗の error ledger 記録、元の取得時刻、新しい targeted
snapshot、queue と明示 refresh の live read は維持する。
実装と既存回帰の変更は issue_cache.rs に限定する。

Goal はこの背景更新の重複取得を止めること。Non-goals / untouched は
UI、queue / prelaunch の live 確認、TTL / deadline / scheduling、
authority、実稼働 fleet の停止・差替え、他の SPEC backlog。
受け入れは実 stale helper の RED/GREEN、既存 cache / hygiene と
fresh canonical pre-pr、Ready Refs PR、必須 CI / auto-merge / develop 包含。
rollback はこの呼出経路修正の revert だけで、migration はない。

Acceptance Surface: backend の REST cache 再利用。描画、対話、UI 出力形式を
変更せず、今回の assertion はプロセス呼出回数・取得時刻・snapshot の保持。
frontend の E2E をこのキャッシュ契約の代替証拠にしない。

### 実稼働 baseline と source の区別

gwt.log.2026-11-10 の20:50:00以上21:00:00未満 JST、600秒を有限集計した。
issue_monitor_scan の project_root が E:\gwt の gh start は113回
（timelines34、issues23、同じ5 PR の lifecycle 各10回、open/closed pulls各3回）。
ログに親 PID はないため GUI の直接子プロセス数とは扱わない。

実行中 GUI は installed v9.157.0、PID20748、起動15:29:09 JST。
GUI SHA-256 は
`BE0BE99D085F76F61D842F9D9CCADF01FDAD60104F00873C7022A9B5E1819B06`。
owner daemon PID25664 は15:29:44 JST 起動、installed gwtd SHA-256 は
`0287D626F58A2BFC71436BAAD0D84365DD1BD10935B77CE65F5E93A502B9F52C`。
この baseline は今回の source 修正後の値ではない。600秒の連続6 agent
稼働、同条件の5分 / 60分 CPU 比較も今回の集計では証明していない。

AC-1/3 の既配送状態を維持し、AC-2 の20回/10分、AC-4 の6 agent CPU、
AC-5 の時間差は未チェックのまま。単体テストや PR の配送で代用しない。

### 回帰とレビュー

既存の Windows failed-refresh 回帰は変更前に1件 PASS。
同じ fixture に scratch Git remote を設定し、Monitor 失敗後の retry と
成功を本物の stale-only helper へ置き換えると、旧実装で general page
取得が3回（期待1回）となり RED。queue 不可用時の失敗と stale marker、
新しい targeted snapshot の保持、queue 3回・明示 refresh 後の general
2回 / queue 4回という既存の assertion は維持した。

修正は stale-only helper の既存 page reuse 接続と full-refresh 失敗記録の
共用。Monitor の budget deferral 例外と明示 refresh は変えない。
独立した確定差分レビューは blocker0。

修正後の Windows issue_cache 全21件が PASS（retries0）。拡張した回帰も
general1回、queue3回、明示 refresh 後general2回 / queue4回で GREEN。
full-refresh 失敗の ledger、Monitor budget deferral の例外、SPEC comment
取得、targeted refresh と freshness metadata の既存回帰も成功した。

コミット前 canonical pre-pr `vrr-a477dad05e154967a5aadd6cd8efe182` は
7工程 PASS（登録6工程と runner 自身の driver 復元）。cache21件・hygiene37件
成功、maintenance helper1件は既存の skip。fmt、変更crateの Clippy、通常
driver 復元も成功。これは最終 HEAD の証跡ではなく、配送時に fresh 記録と
必須 CI を確認する。ローカル full coverage は CI-backed pre-pr 方針で未測定。

Launch mode: autonomous

User Verification Result: n/a (autonomous)

Agent Visual Check: n/a (no UI surface)

## 実ログ

installed v9.155.2 の構造化ログ `gwt.log.2026-11-09` の
`gwt.process.summary` / `phase=start` を集計した。
`gh` は `span.name=issue_monitor_scan` の行だけを数えた。
新 checkout の修正後 fleet 計測ではない。

- 03:51:18.512170–04:01:18.512170 UTC の scan 由来 `gh` 起動は198回。
  timelines 82回、issues 34回、同じ PR lifecycle は4対象で各16回だった。
  同期間の scan proposal rejection は15件、理由は全て `revision-changed`。
- 03:00–04:00 UTC の全ログで `git cherry` 起動は0回。
  ログに親 PID はないが、全体が0回なので GUI の subset も0回。
  通常の Git metadata read を cherry の起動回数として数えない。

| 10分枠の read | 起動回数 |
| --- | ---: |
| issue timelines | 82 |
| issues | 34 |
| PR lifecycle: PR #6919 / PR #6923 / PR #6611 / PR #6918 | 16 × 4 |
| PR lifecycle: PR #6888 | 8 |
| PR lifecycle: PR #6920 | 1 |
| open pulls page 1 | 5 |
| closed pulls page 1 | 4 |
| 合計 | 198 |

## 根因と最小修正

timer-only scan は最大600秒の既存 read cache を使っていた。しかし
proposal の失効、watchdog、retry deadline 後の再試行は `scan_requested`
を立て、明示要求と同じ fresh read へ昇格していた。

TTL 延長だけではこの迂回は直らず、新しい永続 cache は不要なので、
自動再試行の4箇所を既存の cache 許容 request に変更する。
待機中の明示要求は引き続き優先される。開始済み ScanNow の自動再試行は
最大600秒の cache を使うが、新しい ScanNow と effect 完了後の要求は fresh。
成功した read のみが cache に残り、失敗 read は cache しない。

## 検証記録

- 変更前の既存5テスト: PASS。canonical record
  `vrr-25e48b6497b44adf91441464bf5f882e`、driver 復元も PASS。
- 実 worker の新規回帰: RED。最初の ScanNow を channel で止め、IssueState
  control で revision を変えた後に解放すると、再試行の read max age が
  `None` だった。実測は `(None, None, None)`、期待は
  `(None, Some(600), None)`。wall-clock の短時間順序には依存しない。
- 修正後の関連6テスト: GREEN。実 worker の初回明示要求・自動再試行・
  次の明示要求は `Some((None, Some(600), None))`。
- canonical pre-pr `vrr-76fcbc7a023141d596e25e44e2242e14`: 7工程 PASS。
  daemon/worker 135件、cherry cache 2件、test hygiene 37件 PASS
  （baseline 再生成 helper 1件 ignored）、fmt/clippy と driver 復元も PASS。
  これは commit 前の記録。配送は最終 HEAD の fresh 記録と必須 CI で判定する。
- coverage はローカル未測定。必須 CI の `coverage / Rust Coverage` に
  gwt-core + gwt の90% gate と他 domain の80% gate を委ねる。
- Unix の candidate read marker を待つ既存 Issue #4783 回帰は既に
  `GWT_TEST_BUDGET_ISSUE_MONITOR_PERIODIC_READ_MAX_AGE_MS=0` を設定している。
  今回の再試行分類でも live fake-gh read を維持する。Windows でこの Unix
  専用テストが実行されたとは主張しない。

## Owner の受け入れ条件

| AC | base と実測に対する評価 |
| --- | --- |
| AC-1 | PM の v9.155.2 実測で非対象 project の scan は0件。既配送。 |
| AC-2 | 未達。上記10分198回。今回の retry 修正と配送、更新後 fleet の再計測が必要。 |
| AC-3 | 観測した1時間枠では cherry 0回。既存の tip SHA cache と差分更新を維持し追加 source なし。cold cache 初回計算を0回とは主張しない。 |
| AC-4 | 未達。最新 PM 実測は6 agent 稼働で240.3%。今回の変更後 CPU は未測定。 |
| AC-5 | 未証明。最新 PM 実測は時間経過で再増加。今回の変更後60分の測定はない。 |

CPU と旧 PM 判定の出典は
[owner comment](https://github.com/akiojin/gwt/issues/6877#issuecomment-6731427158)。
単体テストや配送だけで owner 全体を close しない。

## PR #6928 macOS CI の引継ぎ（2026-11-10）

サラ・カトウ / Codex。`origin/develop=1cc2b8ed9ec46e553459987a3183cdb83a827c03`
を merge し、既存の未配信 source と fixture 修正を保全した。

RED は [macOS CI run 42568128843](https://github.com/akiojin/gwt/actions/runs/42568128843)。
`cargo test -p gwt --lib daemon -- --test-threads=1` の479件が成功し、
`routine_retry_fences_a_scan_captured_while_the_control_is_pending` だけが
`routine pending state keeps scan progress alive` で失敗した。
既存の長い native TMPDIR 検証内で、fake-gh 開始 marker が現れなかった。

worker の endpoint cache は blocking thread 間で共有される。
`ScopedGwtHome` は thread-local で `spawn_blocking` へ伝播せず、
list cache は先行テストの結果を再利用し得る。linked PR cache も
owner/repo/Issue をキーとする process-global cache である。
自動再試行を cache 許容にしたことで、fixture の「必ず gh が起動する」
という前提が成立しなくなった。CI の実 cache-hit 自体は記録されていないため、
どの endpoint が hit したかを実測済みとは主張しない。

timeout 延長や cache key の変更ではなく、この既存 fixture だけ
`GWT_TEST_BUDGET_ISSUE_MONITOR_PERIODIC_READ_MAX_AGE_MS=0` に固定する。
0 は read max age を `None` にし、queue-labelled Issue read が必ず
fake-gh を通る。既存の env lock 内で guard を保持し、heartbeat の durable
commit / ACK / single-flight assertion と production TTL は維持する。
独立レビューで blocker はなかった。Windows ではこの Unix 専用 fixture を
実行できないため、GREEN の判定には修正 head の macOS CI 成功が必要となる。

統合後の canonical pre-pr `vrr-483d4d873e5048ad92e52625c766a801` は
7工程 PASS。daemon/worker 136件、cherry cache 2件、test hygiene 37件が成功し、
maintenance helper 1件は ignored。fmt / clippy / checkout driver 復元も成功した。
これは commit 前の記録であり、最終 head の fresh 検証と必須 CI は別途確認する。

Owner の AC-2（更新後 fleet の gh 起動数）、AC-4（6 agent の CPU）、
AC-5（60分比較）は引き続き未証明。今回の CI 修正と source 配送で
達成扱いにせず、Issue #6877 は OPEN を維持する。

## 残存する TTL 迂回（2026-11-10 再開）

サラ・カトウ / Codex。fetch 後の `origin/develop` と HEAD はともに
`56abfcb8341ca12c96ed7a9d17ba8bd408261d91`。PR #6928 の source は
base に配送済みで、今回の開始時点に未配送の production source はなかった。
SPEC #4093 の plan / tasks に T-6877-R6〜R9 を追加し、readback を確認した。

### 実 fleet と source の区別

[relaunch-baseline.json](relaunch-baseline.json) に実測値と由来を保存した。
稼働中の GUI は installed release v9.156.0、PID 33516、起動は
2026-11-09T14:26:55Z。PR #6928 の merge より前から動いているため、
この source の改善後計測にはならない。version の一致だけでは source の
一致を保証できず、実行ファイルの SHA-256 も保存した。

- 2026-11-10T02:45:00Z 以上、02:55:00Z 未満の600秒に、scan span 内の
  `gh` start は139回。timelines 53回、issues 23回、PR lifecycle 54回、
  open pulls 6回、closed pulls 3回。project_root は `E:\gwt` のみ。
- 同じ枠の全ログで `git cherry` start は0回。親 PID はログに無いため、
  gh の全件を GUI 直接子と見なさない。
- 03:03:34Z から約5.054秒の GUI process CPU 差分は7.921875秒、
  1 core = 100% の式で156.74%。RSS 約835MiB / private 約1300MiB。
  この単一サンプルを60分の傾向に置き換えない。
- 別時点の pane snapshot では running implementation agent は6本。
  600秒間の連続6本稼働は証明していない。5分 / 60分の同条件比較も未測定。

### 根因と実装境界

1. worker は `RenewClaim` の正常更新を通常の remote effect 完了と同じ
   fresh scan 要求にしていた。既定300秒の heartbeat が timer / retry の
   600秒 read policy を迂回する。正常かつ現 authority の `Renewed` だけ
   既存の periodic intent に接続する。所有権を失った `NotRenewed`、古い
   receipt、他の effect 完了、明示 ScanNow は fresh のままにする。
2. 共有 REST page の GC は呼出し側の120秒 TTL をディレクトリ全体に適用し、
   別 consumer が600秒 / 900秒まで利用できる page を削除していた。
   読取 freshness は呼出し側のまま、保存期間だけ既存最大 consumer の
   900秒へ分離する。新しい cache schema や namespace は追加しない。

GC は既知形式・valid array・通常ファイル・key/hash 一致・共通 lock の
条件を維持する。期限切れデータを短い reader に返す変更ではない。
reclamation が保持する期間は最大15分へ広がるが、期限切れカーソルの
無期限蓄積には戻さない。Issue #5738 に120秒以内の回収要件はない。

Goal はこの2経路の迂回解消。UI、Issue #6954 の timeline pagination、
hook health、prefs-lock 設計、deadline、claim/permission/launch authority、
live prelaunch confirmation は対象外。実 fleet の停止・再起動は行わない。
rollback はこの2箇所の policy 修正を戻せばよく、データ migration は不要。

### 決定的な再現結果

- 修正前の既存 REST cache 14件と自動 retry worker 1件は成功。
- mixed-TTL 回帰は修正前に `(Monitor5回, 通常5回)` で失敗した。
  30秒間隔20scan / 600秒を仮想時刻で再現し、修正後は `(1回, 5回)`。
  Monitor page の再取得は80%減り、通常 reader の120秒 freshness は維持。
  関連16件すべて成功。既存900秒 refresh の取得時刻・期限切れの失敗伝播・
  invalid payload・queue pagination・安全な既知page回収も成功した。
- actual worker の claim 更新回帰は修正前に `(None, None)` で失敗した。
  正常更新後600秒、続く明示 ScanNow は fresh という期待値は
  `(Some(600), None)`。channel で scan と control の順序を固定し、
  スケジューラの短時間応答に依存しない。
  修正後はこの正常更新 case と `NotRenewed=(None, None)` の両方が成功した。

この replay の fetch 回数は実 fleet の gh 子プロセス数ではない。
AC-2 の20回/10分、AC-4 の6 agent CPU、AC-5 の5分 / 60分比較は
更新後の同一実行ファイルで別途測定する。単体テスト成功を達成証拠に
置き換えず、owner の未チェックを維持する。

User Verification Result: n/a (autonomous)

Agent Visual Check: n/a (no UI surface)

## 2026-11-11: branch merge verdict の lineage スラッシング（サクラ）

### 観測（installed GUI PID 34596、v9.159 系、再起動後約 30 分）

- `~/.gwt/logs/gwt.log.2026-11-11` の `gwt.process.summary` を 12:45〜12:55
  の 10 分で集計すると、GUI が起動した git/gh は 1,429 本。上位は
  `rev-parse --verify --quiet origin/develop` 398、`merge-base --is-ancestor`
  252、`rev-parse --verify --quiet work/issue-N` 214、
  `rev-parse work/issue-N^{commit} origin/develop^{commit}` 196、
  `git cherry` 17（AC-3 の 10 回/10 分を超過）。
- この並びは branch list hydrate（`build_cleanup_targets` →
  `detect_cleanable_target_with_remote_names` → `is_branch_merged_into`）の
  ブランチ 1 本ごとの手順と一致する。1 回の hydrate が約 130 本の
  ローカルブランチを 1〜3 秒/本で処理し、10 分間途切れず続いていた。
- 古いブランチの `merge-base --is-ancestor` は 1 回 0.4〜1.5 秒、
  `git cherry` は約 2 秒。
- `branch-merge-verdicts.json` の 51da5cd（未マージ）の lineage は
  `origin/develop` 現 tip と `origin/main` の tip 2 つ（8b46369 / 46b11ca）。
  ログでは develop 判定のたびに main tip の lineage を `merge-base` で
  順に試し、全て不一致のあと `git cherry` を全量で再実行していた。

### 根因

`is_merged_cached` は base が進んだとき、祖先にあたる lineage を
**取り外して**新しい base へ付け替えていた。release 後の `origin/main` は
古い `origin/develop` tip を含むため、main 判定が develop の lineage を
消費する。次の develop 判定は使える lineage が無く `git cherry` を
全量実行し、main 判定がまたそれを消費する。キャッシュがあっても、
未マージのブランチは毎回 base ごとに cherry を払っていた。さらに
`is_branch_merged_into_existing_refs` は cache を見る前に毎回
`merge-base --is-ancestor <branch> <base>` を起動していた。

### 修正（`crates/gwt-git/src/merge_verdict_cache.rs` / `branch.rs`）

1. 進めた元の lineage は残し、新しい base の lineage を先頭に足す
   （1 tip あたり最大 4 本は既存どおり）。
2. lineage の祖先判定（base tip 同士）はプロセス内でメモ化する。commit の
   祖先関係は不変で、1 回の hydrate の全ブランチが同じ base の組を問う。
   エラーはメモ化しない。
3. ブランチ自身の `--is-ancestor` は lineage が無い tip だけで実行する。
   base が進んだ範囲の `rev-list --no-merges` にはマージコミットで入った
   ブランチ自身のコミットも含まれるので、patch id の照合で検出できる。

RED: 新規テスト
`is_branch_merged_into_keeps_one_lineage_per_base_across_releases` が
修正前に失敗した（release 後、tip の lineage が main 2 つだけ）。GREEN 後は
develop / main の lineage が両方残り、変化が無い組の再判定は ref 確認 2 本と
`rev-parse` 1 本の計 3 spawn だけになる（修正前は `merge-base` を含め 4 本）。
gwt-git lib 425 件と `merged_branch_prune_test` 14 件が成功。

### 範囲外 / 未測定

- 実 fleet の AC-2（gh ≤20/10 分）、AC-4（6 agent で 5 秒 CPU ≤100%）、
  AC-5（5 分 / 60 分の差 ≤20pt）は、この修正を含む実行ファイルで
  PM が測定する。単体テストの結果を fleet の達成証拠にはしない。
- 同じ 20 秒採取で、GUI 直下に `git status --porcelain=v1
  --untracked-files=all --ignored=traditional -- .gwt/work/events.jsonl
  .gwt/work/events` が 9 本あった（`work_event_dirty_paths`、Work event
  settlement の判定）。これは `hidden_command` 経由で
  `gwt.process.summary` に出ない。コマンドラインが空の git 16 本も
  記録経路が同じと見られ、caller は特定できていない。この slice では
  変更しない。

## 2026-11-11: Reviewing gate の毎 scan readback（サクラ、次スライス）

### 観測（installed GUI PID 34596、14:32〜14:42 JST の 10 分）

`gwt.process.summary` の gh 起動は E:\gwt 139 本、E:\unity-cli 116 本で、
AC-2（1 プロジェクト 10 分で 20 本以下）を満たしていない。すべて
`issue_monitor_scan` span 内。上位は Reviewing 中の PR 1 本あたり scan ごとに
4 本ずつ出る gate readback で、unity-cli PR #556 は
`branches/main/protection` / `pr view --json statusCheckRollup` /
`--json headRefOid` / `--json state,mergeable,mergeStateStatus` が各 26 本、
gwt PR #7047 も同じ 4 本が各 19〜21 本だった。

### 修正（`crates/gwt/src/issue_monitor_worker.rs` / `crates/gwt-git/src/pr_status.rs`）

1. rollup・head・mergeability を `gh pr view --json
   statusCheckRollup,headRefOid,state,mergeable,mergeStateStatus` の 1 本で読む。
2. branch protection を既存の in-process read cache（timer scan の read max
   age 内だけ有効、失敗はキャッシュしない）で再利用する。
3. gate が「必須 CI 実行中」（WaitForCi）と判定した PR は、timer scan では
   60 秒間 gate readback を読み直さない。手動 scan は従来どおり毎回読む。
   Deliver / Remediate は常に新しい readback で判定する。

PR 1 本が 10 分 Reviewing に留まる場合、gh は約 104 本から約 11 本になる見込み。

### 範囲外 / 未測定

- E:\gwt の `gh api graphql issue timelines`（24 本）と `gh api issues`
  （20 本）は Reviewing gate とは別経路で、この slice では変えていない。
  そのため gwt プロジェクトでは修正後も AC-2 の 20 本に届かない見込み。
- AC-4 / AC-5 は実 fleet で測定する。

## 2026-11-12: corrected-head 待ちの個別再読（サチ）

分類は gwt runtime の性能修正。fetched base と開始時 HEAD はともに
`9221b0c75953e23eae442dc0ff0b73ad822b53a9`。branch-only commit は0、
既存 PR #7058 は MERGED。owner は Issue #6877、関連 SPEC は Issue #4093。
既配送 source は再実装せず、FR-6877-5 / T-6877-R16〜R18 を追加した。

### 観測と根因

[review-head-baseline.json](review-head-baseline.json) は installed GUI
PID52136（PM記録の v9.160.0、2026-11-11T11:40:47Z 起動）の既存ログを
01:00〜01:10 JST の600秒に限定して集計したもの。gwt project の
`issue_monitor_scan` 内 gh start は67本で、PR #7083 / PR #7080 /
PR #7074 の `headRefOid` が各12本、計36本を占めた。
同時刻の hold reason は全て「corrected head 待ち」であった。
親PIDはログにないため、GUI直接子の厳密な実測とは区別する。

`Implementing` の review ladder / remediation が存在すると、worker は
保留判定より前に毎scan個別HEADを取得する。open-pulls REST page は
同じ scan が既に取得しており `head.sha` も含むが、数値PR一覧に変換した
後の保留判定では使われていなかった。

### 最小 Plan と境界

Goal は保留中の重複HEAD取得の除去。TTL延長、新規cache、既存REST pageの
再利用を比較し、既存page再利用を選ぶ。読取アダプタは同じ endpoint /
`--include` / pager / cache key を共有する。periodic scan の発見情報は
最大600秒遅れるが、実review dispatchは live HEAD / diff と既存の
`dispatch_review` による最終hold再検査を維持する。manual scanもlive。

対象は `pr_status.rs` / `issue_monitor_worker.rs` の読取経路と既存回帰。
Non-goals / untouched は UI、TTL・deadline値、review ladder、slotと
mutationのauthority、稼働fleetの停止・差替え、他の SPEC backlog。
rollback はこの読取変更のrevertで、永続形式やmigrationの変更はない。

受け入れは既存inventory / backoffテスト、実worker fixtureのRED/GREEN、
cacheとliveで異なるSHAのdispatch、manual live、hygiene、fresh canonical
pre-pr、Ready Refs PR、必須CI / auto-merge / develop包含で判定する。
same-headのbackoff経過は既存domain回帰も併用して確認する。

この36本を除くだけでも、元の観測には他経路31本が残る。fixtureでの
呼出回数低減をfleetのAC-2（20本以下）へ置き換えない。AC-1/3の既配送
状態を維持し、AC-2/4/5は更新後の適合する実測まで未チェックとする。
PMのCPU観測は12点中11点が100%以下、3分/58分の差15ptであり、厳密な
6 agent・5分/60分条件の全証明とは区別する。

残るtimeline14本は成功13・timeout1。成功直後の13ログは全て
`pending=true`、延べ完了数32で、既存cursor継続とbacklog処理に整合する。
ただし対象Issue番号・cursor・query識別子・read max ageがログにないため、
unique完了数や同一対象再読の内訳は確定できない。31本は修正後の予測値
でもない。scan頻度が変わり得るため、別途実測が必要である。

### 回帰とレビュー

変更前のinventory4件と既存worker backoff1件はPASS。拡張fixtureは
初回+保留2scanでHEAD取得3回、期待1回となってREDを確認した。
先行したWindowsログ引用符によるcount0はテスト側の集計ミスとして修正し、
production変更前に期待した原因でREDを再確認している。

GREENの`issue_4815_` 5件はPASS。保留2scanのHEAD個別取得は0回、
共有pulls pageは1回だけ取得。cacheのabc123とliveのdef456を分け、
実dispatchがdef456へ結びつくこと、次のmanual scanがlive読取することを
確認した。既存domainテストでsame-head backoff経過後の再試行と
3失敗時の停止・新HEADによる解除も維持した。実差分の独立レビューは阻害0。

Acceptance Surface: non-user-facing (remote discovery/cache routing only)。

User Verification Result: n/a (autonomous)

Agent Visual Check: n/a (no UI surface)
