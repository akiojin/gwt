# Issue #5503: usage poll の増分集計の計測

2026-10-15、Windows、debug build。比較元は develop `c5b16dc00b40e6dd8fbb4a66b5c1d3802e779f11` の従来収集経路。今回の同一テスト内で従来処理を再現し、修正後の実際の `Poller::poll_once` と比較した。原出力は [poll-measurements.txt](poll-measurements.txt)。

## 条件と測定範囲

- 生成した ledger 2,646行、うち32 active session（Codex/Claude 各16）、JSONL 合計33,598,656 bytes。実ユーザーの transcript・資格情報は読まず、Claude account HTTP は無効。
- 同一fixture・同一収集threadで各ケース3回。ledgerの共有cacheは双方ともwarm。coldはtranscript cacheだけを毎回作り直す。
- wallは各pollの開始から終了まで。CPUは実際に収集したthreadの `GetThreadTimes` 差分であり、別threadへの移動だけをCPU削減と数えない。WindowsのCPU計測粒度と、並行ビルド等のhost負荷の影響を受ける。
- settings、account、ledger、session、consumption を含む収集本体を比較する。WebSocket配送とaccount snapshotのファイルpublish、blocking poolへの待ち時間は計測対象外。全体がblocking pool上で実行されることは別のthread ID回帰で固定する。
- bytesはtranscriptのみ。従来は呼び出した全文readerのfixture sizeを合計し、修正後は実read counterを使用。auth/settings/ledgerのbytesは双方とも含まない。
- 通常pollでは既存の300秒consumption cacheを使い、forcedでは再集計する。new appendedは最初の1回だけ2ファイルへ追記し、残り2回は未変更。

## 結果

wall/CPUは1 pollの平均ms、bytesは3回の合計。

| ケース | wall ms | 収集thread CPU ms | transcript bytes |
| --- | ---: | ---: | ---: |
| 従来・通常poll | 495.817 | 442.708 | 126,013,200 |
| 従来・消費量も再収集 | 759.884 | 677.083 | 226,809,168 |
| 修正後・transcript cache cold | 918.162 | 458.333 | 100,795,968 |
| 修正後・通常poll | 223.422 | 145.833 | 0 |
| 修正後・消費量も再収集 | 253.739 | 151.042 | 0 |
| 修正後・2ファイルへ追記 | 207.453 | 171.875 | 552 |

通常pollのwallは 54.94%、収集thread CPUは 67.06% 減少した。未変更の通常/forced pollの本文読取は0 byte。追記ケースの合計552 bytesは新しい2行だけであり、以後の再pollは0 byteだった。

coldのwallは従来forcedより遅い実測で、cold wallの改善は主張しない。全ケースでaccount/session/consumptionを含む完全なsnapshotが従来処理と一致した。時間の閾値assertは置かず、同値性と読取bytesを回帰条件にする。

## 検証と境界

- 再現コマンド: `cargo test -p gwt --bin gwt usage_poller::performance_tests:: -- --ignored --nocapture --test-threads=1`。
- 関連回帰: `cargo test -p gwt-core --lib usage:: -- --test-threads=1`、`cargo test -p gwt --bin gwt usage_poller:: -- --test-threads=1`。
- UI維持確認: `pwsh -NoProfile -File scripts/verify-issue-5503.ps1 --headed`。このcheckoutのgwt/gwtdとfresh HOMEで既存provider-usageをdark/lightで実行し、console/page errorも検査する。正式結果は同じ配送HEADのverify.run記録とPR本文に記載する。
- provider JSONLの追記形式を前提とする。同じinode/file identityのまま長くなる上書きは追記と区別しない。切詰め、同サイズ書換（mtime変化）、ファイル置換、欠落は集計を無効化する。部分JSON/UTF-8と改行なしの完全JSONは重複計上しない。
- event cacheは解析済みtokenイベントのみ保持する。entryは現在のaccount/active sessionと直近のconsumption scanに絞り、transcript本文全体は保持しない。
- この生成fixtureの比較は実fleet全体のCPU/RSS保証ではない。配布後の8窓・60分計測はIssue #5385のまま残す。workspace coverageはローカル未測定、正式gateは同じPR headの必須CI 90%/80%へ委ねる。

User Verification Result: n/a (autonomous)
