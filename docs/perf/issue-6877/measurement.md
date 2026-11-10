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
