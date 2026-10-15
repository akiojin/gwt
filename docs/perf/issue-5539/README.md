# Issue #5539: 実検索の baseline

Windows の debug build、production source commit `8e792dee7be82c8565ac8fba3d4feeae889a9ade` を計測した。対象は run `dde8f33ca362479f9bbf10712b1594a5` のみ。バイナリ、production source、runtime runner、検証 harness の SHA-256 は [checkout-evidence.json](baseline/checkout-evidence.json) に保存した。この baseline は計測追加後、Rust から resident worker への直接接続を追加する前の経路である。

fresh HOME に memory 文書を 1 件だけ置き、実モデル `intfloat/multilingual-e5-base` を既存キャッシュから offline で読み込んだ。Python 依存だけを共有し、index・coordinator・resident worker は隔離した。PM・Issue Monitor・バックグラウンド索引は停止した。実モデルの事前ウォームアップ後、最初の GUI 検索を測定対象から除外し、同一の `memory` query を 20 回逐次送信した。その後、別々の request ID で 8 件を同時送信し、検索が pending の間の settings・preset list 応答と Health タブ操作を検証した。各テーマで最後に実 UI 入力からの検索成功も確認した。

実 Chromium の headed 実行で dark、light の順に検証した。両テストは成功し、console/page error は 0 件だった。[Playwright 結果](baseline/playwright-last-run.json)。baseline では p95 の閾値失敗を許可しているため、テスト成功は両テーマの p95 が 2 秒以内だったことを意味しない。

| テーマ | 逐次件数 | wall 平均 ms/検索 | wall p95 ms | GUI CPU 合計 s | GUI CPU s/検索 | CPU 測定窓 s | GUI 平均 CPU %（1 core 基準） |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| dark | 20 | 1558.97 | 2947.72 | 30.390625 | 1.519531 | 36.346 | 83.61 |
| light | 20 | 1369.89 | 1801.61 | 8.250000 | 0.412500 | 29.940 | 27.56 |

wall は Node 側で送信処理を始めてから、実 WebSocket の対応する結果フレームを受信するまでの時間で、Playwright の送信処理も含む。p95 は 20 件を昇順に並べた 19 番目。GUI CPU は同一 PID・開始時刻の `Get-Process.TotalProcessorTime` の差分で、測定窓内の全 GUI thread の活動を含む。CPU s/検索はその合計を 20 で割った値、平均 CPU % は合計 CPU 秒を測定窓の wall 秒で割った値である。検索専用の CPU 帰属を示さず、別プロセスの Python worker や子プロセスの CPU は含まない。CPU 測定窓にはサンプラーの起動・終了間隔も含む。

同じ実行の fresh HOME にある perf log から、各テーマの逐次 CPU snapshot 区間内だけを抽出した段階別平均を以下に示す。各セルは 20 samples の平均 ms。[集計と区間](baseline/stage-means.json)、[元の search perf samples](baseline/perf-search.jsonl)。

| perf target | dark ms | light ms |
| --- | ---: | ---: |
| `phase:search.git_context` | 567.99 | 456.86 |
| `phase:search.runtime` | 15.79 | 21.31 |
| `phase:search.scope_context` | 2.09 | 2.04 |
| `phase:search.search_and_repair` | 966.78 | 868.23 |
| `phase:search.assemble` | 0.04 | 0.04 |
| `phase:search.runner.index_read` | 83.85 | 67.89 |
| `phase:search.runner.model_load` | 0.00 | 0.00 |
| `phase:search.runner.query_encode` | 85.39 | 68.00 |
| `phase:search.runner.query` | 64.40 | 68.22 |
| `phase:search.runner.assemble` | 0.01 | 0.01 |
| `route:search` | 1552.97 | 1348.75 |

runner の段階は `search_and_repair` の内側の値なので、全行を加算しない。`route:search` は Rust 側の区間であり、上表のブラウザ往復 wall と測定境界が異なる。生データには warmup・逐次・同時・UI 入力による計 60 件の route samples と対応する search phase samples を保持した。逐次平均には各テーマ 20 件だけを使用した。

| テーマ | 実送信 | 同時検索の成功 / unavailable / 未応答 | settings 応答 ms | preset list 応答 ms | Health 操作 ms |
| --- | ---: | --- | ---: | ---: | ---: |
| dark | 8 | 6 / 2 / 0 | 11.78 | 11.84 | 72.80 |
| light | 8 | 8 / 0 / 0 | 8.72 | 8.76 | 89.45 |

いずれも control 応答・Health 操作時点で 8 件が pending だった。観測上限は 45 秒で、この実行では未応答を残さず観測を終えた。dark の unavailable 2 件は成功に置き換えていない。[dark 生データ](baseline/baseline-dark-search-route.json)、[light 生データ](baseline/baseline-light-search-route.json)。GUI 応答の受け入れ条件と、同時検索すべての配送成功は分けて扱う。

この結果は 1 文書・同一 query の fixture に限定され、通常の全 corpus の baseline ではない。テーマは同一 GUI で順に実行し、host の他負荷を固定・計測していないため、テーマ間の CPU 差や段階時間から原因を断定しない。比較後の因果や性能改善率はこの baseline 単体では判断できない。実モデルの cold load は [model-prewarm.json](baseline/model-prewarm.json) に別記し、索引 1 件の作成結果は [index-memory.json](baseline/index-memory.json) に保存した。

User Verification Result: n/a (autonomous)
