# Issue #5539: 実検索の測定

Windows の debug build で以下の 3 実行を計測した。最終 after は dark / light とも p95 が 2 秒以内となり、厳格な headed テストが成功した。native 接続のみの中間試行は **dark の p95 が基準を超えた失敗試行**として残し、AC-2 の合格証跡には使わない。バイナリ、production source、runtime runner、検証 harness の SHA-256 は各実行の metadata に保存した。Playwright テストの SHA-256 は 3 実行で一致する。最終 launcher の変更は `index_worker.rs` のハッシュを metadata へ追加するものだけで、測定方法は変えていない。

| 区分 | production source commit | run ID | metadata |
| --- | --- | --- | --- |
| baseline（計測追加後、Rust の直接接続追加前） | `8e792dee7be82c8565ac8fba3d4feeae889a9ade` | `dde8f33ca362479f9bbf10712b1594a5` | [metadata](baseline/checkout-evidence.json) |
| native 接続のみの試行 | `0067510ba7da982736575e28a73a0738d91e6f8b` | `d27d8353e2d04452bca8d7db57f82272` | [metadata](native-only-attempt/checkout-evidence.json) |
| 最終 after（native 接続 + Git metadata の狭い fast path） | `91800263ecc62b2480b9b859a032040d9336b2bc` | `4a7ca315866b492e99cda61396a6f05e` | [metadata](after/checkout-evidence.json) |

各実行の fresh HOME に memory 文書を 1 件だけ置き、実モデル `intfloat/multilingual-e5-base` を既存キャッシュから offline で読み込んだ。Python 依存だけを共有し、index・coordinator・resident worker は隔離した。PM・Issue Monitor・バックグラウンド索引は停止した。実モデルの事前ウォームアップ後、最初の GUI 検索を測定対象から除外し、同一の `memory` query を 20 回逐次送信した。その後、別々の request ID で 8 件を同時送信し、検索が pending の間の settings・preset list 応答と Health タブ操作を検証した。

実 Chromium の headed 実行で dark、light の順に検証した。baseline は p95 の閾値失敗を許可し、両テストが成功した。[baseline 結果](baseline/playwright-last-run.json)。native 試行では閾値を厳格に適用し、dark が p95 2533.966 ms で失敗、light が成功した。[native 試行の結果](native-only-attempt/playwright-last-run.json)。最終 after は閾値を厳格に適用し、両テーマが成功した。[最終 after 結果](after/playwright-last-run.json)。console/page error は全実行・両テーマで 0 件だった。最後の実 UI 入力による検索成功は baseline と最終 after の両テーマ、native 試行の light で確認した。native 試行の dark は p95 assertion でその前に終了したため、実 UI 入力の成功を確認済みとはしない。

| 区分 / テーマ | 逐次件数 | wall 平均 ms/検索 | wall p95 ms | GUI CPU 合計 s | GUI CPU s/検索 | CPU 測定窓 s | GUI 平均 CPU %（1 core 基準） |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| baseline / dark | 20 | 1558.97 | 2947.72 | 30.390625 | 1.519531 | 36.346 | 83.61 |
| native 試行 / dark | 20 | 1627.81 | 2533.97 | 32.328125 | 1.616406 | 36.615 | 88.29 |
| 最終 after / dark | 20 | 551.37 | 924.74 | 12.421875 | 0.621094 | 15.465 | 80.32 |
| baseline / light | 20 | 1369.89 | 1801.61 | 8.250000 | 0.412500 | 29.940 | 27.56 |
| native 試行 / light | 20 | 1084.65 | 1927.83 | 7.453125 | 0.372656 | 25.826 | 28.86 |
| 最終 after / light | 20 | 504.99 | 740.53 | 12.765625 | 0.638281 | 15.121 | 84.42 |

wall は Node 側で送信処理を始めてから、実 WebSocket の対応する結果フレームを受信するまでの時間で、Playwright の送信処理も含む。p95 は 20 件を昇順に並べた 19 番目。GUI CPU は同一 PID・開始時刻の `Get-Process.TotalProcessorTime` の差分で、測定窓内の全 GUI thread の活動を含む。CPU s/検索はその合計を 20 で割った値、平均 CPU % は合計 CPU 秒を測定窓の wall 秒で割った値である。検索専用の CPU 帰属を示さず、別プロセスの Python worker や子プロセスの CPU は含まない。CPU 測定窓にはサンプラーの起動・終了間隔も含む。

各実行の fresh HOME にある perf log から、各テーマの逐次 CPU snapshot 区間内だけを抽出した段階別平均を以下に示す。セルは平均 ms（記録された sample 数）。baseline の [集計](baseline/stage-means.json)・[生データ](baseline/perf-search.jsonl)、最終 after の [集計](after/stage-means.json)・[生データ](after/perf-search.jsonl)。

| perf target | baseline dark | 最終 after dark | baseline light | 最終 after light |
| --- | ---: | ---: | ---: | ---: |
| `phase:search.git_context` | 567.99 (20) | 2.23 (20) | 456.86 (20) | 2.65 (20) |
| `phase:search.runtime` | 15.79 (20) | 34.09 (20) | 21.31 (20) | 21.50 (20) |
| `phase:search.scope_context` | 2.09 (20) | 14.35 (20) | 2.04 (20) | 2.40 (20) |
| `phase:search.search_and_repair` | 966.78 (20) | 493.89 (20) | 868.23 (20) | 466.65 (20) |
| `phase:search.admission` | 未計測 | 37.07 (20) | 未計測 | 18.69 (20) |
| `phase:search.runner_transport` | 未計測 | 424.87 (20) | 未計測 | 409.60 (20) |
| `phase:search.assemble` | 0.04 (20) | 0.04 (20) | 0.04 (20) | 0.04 (20) |
| `phase:search.runner.index_read` | 83.85 (20) | 156.89 (20) | 67.89 (20) | 173.76 (20) |
| `phase:search.runner.model_load` | 0.00 (20) | 0.00 (20) | 0.00 (20) | 0.00 (20) |
| `phase:search.runner.query_encode` | 85.39 (20) | 105.27 (20) | 68.00 (20) | 85.95 (20) |
| `phase:search.runner.query` | 64.40 (20) | 160.72 (20) | 68.22 (20) | 147.78 (20) |
| `phase:search.runner.assemble` | 0.01 (20) | 0.01 (20) | 0.01 (20) | 0.01 (20) |
| `route:search` | 1552.97 (20) | 544.90 (20) | 1348.75 (20) | 493.52 (20) |

runner・admission・runner_transport の段階は `search_and_repair` の内側の値なので、全行を加算しない。`runner_transport` は worker の計算と応答待ちを含む壁時計時間で、純粋な通信コストではない。`route:search` は Rust 側の区間であり、上表のブラウザ往復 wall と測定境界が異なる。baseline と最終 after は warmup・逐次・同時・UI 入力による各 60 件、native 試行は dark の UI 入力分を除く 59 件の route samples と search phase samples を保持した。

native 試行の dark では、全 20 件が記録された `git_context` の平均 924.09 ms が最大の外側段階で、`route:search` 平均 1604.83 ms の 57.6% を占めた。最終 after の同段階は dark 2.23 ms、light 2.65 ms となった。最終 after で最大の外側 wall 区間は `search_and_repair` の平均 493.89 / 466.65 ms で、その内側の `runner_transport` は平均 424.87 / 409.60 ms だった。残る時間が worker の読み取り・query encode・query と応答待ちを含む区間にある、と測定境界を限定する。GUI の CPU 時間を特定の段階へ帰属させる根拠にはしない。

native 試行の [集計](native-only-attempt/stage-means.json)・[生データ](native-only-attempt/perf-search.jsonl) も保持した。collector は自己予算に応じて sample を間引くため、native 試行の light は段階ごとの記録件数が異なる。`admission` は 26.34 ms（20 samples）、`runner_transport` は 492.25 ms（12）、`git_context` は 719.58 ms（13）。欠けた値を補完せず、段階平均の和や全体比は計算しない。

| 区分 / テーマ | 実送信 | 同時検索の成功 / unavailable / 未応答 | settings 応答 ms | preset list 応答 ms | Health 操作 ms |
| --- | ---: | --- | ---: | ---: | ---: |
| baseline / dark | 8 | 6 / 2 / 0 | 11.78 | 11.84 | 72.80 |
| native 試行 / dark | 8 | 8 / 0 / 0 | 10.85 | 10.90 | 463.91 |
| 最終 after / dark | 8 | 8 / 0 / 0 | 12.35 | 12.42 | 190.20 |
| baseline / light | 8 | 8 / 0 / 0 | 8.72 | 8.76 | 89.45 |
| native 試行 / light | 8 | 8 / 0 / 0 | 281.79 | 281.84 | 112.14 |
| 最終 after / light | 8 | 8 / 0 / 0 | 10.20 | 10.24 | 122.84 |

いずれも control 応答・Health 操作時点で 8 件が pending だった。観測上限は 45 秒で、全実行が未応答を残さず観測を終えた。baseline dark の unavailable 2 件は成功に置き換えていない。baseline の [dark 生データ](baseline/baseline-dark-search-route.json)・[light 生データ](baseline/baseline-light-search-route.json)、native 試行の [dark 生データ](native-only-attempt/after-dark-search-route.json)・[light 生データ](native-only-attempt/after-light-search-route.json)、最終 after の [dark 生データ](after/after-dark-search-route.json)・[light 生データ](after/after-light-search-route.json)。GUI 応答の受け入れ条件と、同時検索すべての配送成功は分けて扱う。

この結果は 1 文書・同一 query の fixture に限定され、通常の全 corpus の baseline ではない。テーマは同一 GUI で順に実行し、host の他負荷を固定・計測していないため、テーマ間の CPU 差や実行間の変化から原因を断定しない。最終 after はこの fixture の wall 平均・p95 が両テーマで低くなった一方、GUI CPU 合計は dark で減り light で増えた。全 corpus の速度や、検索専用・fleet 全体の CPU 削減率を示したとはしない。実モデルの cold load は各実行の `model-prewarm.json`、索引 1 件の作成結果は `index-memory.json` に別記した。

User Verification Result: n/a (autonomous)

Agent Visual Check: pass (final after; real headed Chromium, dark/light, console/page error 0)
