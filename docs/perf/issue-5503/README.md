# Issue #5503: usage poll の増分集計の計測

2026-10-15、Windows、debug build。比較元は develop `c5b16dc00b40e6dd8fbb4a66b5c1d3802e779f11` の従来収集経路。今回の同一テスト内で従来処理を再現し、修正後の実際の `Poller::poll_once` と比較した。原出力は [poll-measurements.txt](poll-measurements.txt)。

最終計測はreview修正後の共通exact filename matcherを両readerへ適用した状態で行った。基準の全文収集経路は上記develop時点と同じで、ID判定方法の差は比較に含めない。

## 条件と測定範囲

- 生成した ledger 2,646行、うち32 active session（Codex/Claude 各16）、JSONL 合計33,599,424 bytes。実ユーザーの transcript・資格情報は読まず、Claude account HTTP は無効。
- 同一fixture・同一収集threadで各ケース3回。ledgerの共有cacheは双方ともwarm。coldはtranscript cacheだけを毎回作り直す。
- wallは各pollの開始から終了まで。CPUは実際に収集したthreadの `GetThreadTimes` 差分であり、別threadへの移動だけをCPU削減と数えない。WindowsのCPU計測粒度と、並行ビルド等のhost負荷の影響を受ける。
- settings、account、ledger、session、consumption を含む収集本体を比較する。WebSocket配送とaccount snapshotのファイルpublish、blocking poolへの待ち時間は計測対象外。全体がblocking pool上で実行されることは別のthread ID回帰で固定する。
- bytesはtranscriptのみ。従来は呼び出した全文readerのfixture sizeを合計し、修正後は実read counterを使用。auth/settings/ledgerのbytesは双方とも含まない。
- 通常pollでは既存の300秒consumption cacheを使い、forcedでは再集計する。new appendedは最初の1回だけ2ファイルへ追記し、残り2回は未変更。

## 結果

wall/CPUは1 pollの平均ms、bytesは3回の合計。

| ケース | wall ms | 収集thread CPU ms | transcript bytes |
| --- | ---: | ---: | ---: |
| 従来・通常poll | 465.413 | 458.333 | 126,016,080 |
| 従来・消費量も再収集 | 667.833 | 614.583 | 226,814,352 |
| 修正後・transcript cache cold | 469.842 | 401.042 | 100,798,272 |
| 修正後・通常poll | 186.809 | 156.250 | 0 |
| 修正後・消費量も再収集 | 301.885 | 203.125 | 0 |
| 修正後・2ファイルへ追記 | 172.461 | 114.583 | 558 |

通常pollのwallは 59.86%、収集thread CPUは 65.91% 減少した。未変更の通常/forced pollの本文読取は0 byte。追記ケースの合計558 bytesは新しい2行だけであり、以後の再pollは0 byteだった。

coldは毎回transcript cacheを再構成した値で、ledgerはwarmのままである。起動全体の速度を示す値ではない。全ケースでaccount/session/consumptionを含む完全なsnapshotが従来処理と一致した。時間の閾値assertは置かず、同値性と読取bytesを回帰条件にする。

## 検証と境界

- 完全一致のsession locatorとcached/stateless同値性を使用量回帰で固定する。既存source-inventory回帰も `cargo test -p gwt --bin gwt embedded_server::tests::transport_direct_dispatch_inventory_stays_explicit -- --exact` で確認する。

- 再現コマンド: `cargo test -p gwt --bin gwt usage_poller::performance_tests:: -- --ignored --nocapture --test-threads=1`。
- 関連回帰: `cargo test -p gwt-core --lib usage:: -- --test-threads=1`、`cargo test -p gwt --bin gwt usage_poller:: -- --test-threads=1`。
- UI維持確認: `pwsh -NoProfile -File scripts/verify-issue-5503.ps1 --headed`。このcheckoutのgwt/gwtdとfresh HOMEで既存provider-usageをdark/lightで実行し、console/page errorも検査する。正式結果は同じ配送HEADのverify.run記録とPR本文に記載する。
- provider JSONLの追記形式を前提とする。同じinode/file identityのまま長くなる上書きは追記と区別しない。切詰め、同サイズ書換（mtime変化）、ファイル置換、欠落は集計を無効化する。部分JSON/UTF-8と改行なしの完全JSONは重複計上しない。
- event cacheは解析済みtokenイベントのみ保持する。entryは現在のaccount/active sessionと直近のconsumption scanに絞り、transcript本文全体は保持しない。
- この生成fixtureの比較は実fleet全体のCPU/RSS保証ではない。配布後の8窓・60分計測はIssue #5385のまま残す。workspace coverageはローカル未測定、正式gateは同じPR headの必須CI 90%/80%へ委ねる。

User Verification Result: n/a (autonomous)
