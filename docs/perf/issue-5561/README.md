# Issue #5561: Work ingest の source map 保持量

## 対象と原因

installed v9.123.2 の GUI（PID 36240、2026-10-15T14:11:46Z 起動）で private memory の増加を観測した。読み取り専用の private-region サンプルでは、約865MBの検査範囲に `.jsonl` が984,543回、`.gwt` が1,070,144回現れた。[サンプル](before-installed/private-region-sample.json) は内容を出力せず、固定マーカーと領域サイズを集計したもの。解放済みの heap page も含むため、これだけを live leak の証明には使わない。

実データは495 worktree、1,232 fingerprint group、2,430,572 source、group file 約484MBだった。[規模](before-installed/source-counts.json)。変更前の Work ingest は、変更グループの全 source map と、比較のために読み込んだ旧 map を処理終了まで保持し、最終確認でさらに旧 map を2回複製していた。

製品コードの変更は既存の2ファイルに限定する。まず `ScannedGroup` を件数と length-delimited SHA-256 に変え、比較済みの正常な旧 map を解放した。dirty な移行データと読込失敗は保持し、最終確認は追加なしで複製0回、追加ありで1回とした。

## 同一実データの割当量比較

稼働中の advisory fingerprint store を `target` 配下へ複製し、同じ2,430,572 sourceを別プロセスで再検証した。[入力](allocation/input.json)、[計測器・バイナリ識別](allocation/metadata.json)、[旧処理](allocation/old.json)、[新処理](allocation/new.json)。対象は全グループの metadata-only 再検証で、payload intake、projection fold、永続化、GUI は含まない。

| 計測値 | 旧保持処理 | 修正後 |
| --- | ---: | ---: |
| 最終確認後の process live heap | 1,371,713,318 B | 681,873 B |
| peak live heap | 1,376,426,952 B | 5,269,500 B |
| 観測最大 WS | 1,601,781,760 B | 18,067,456 B |
| 累積割当要求 | 3,228,569,803 B | 3,028,537,248 B |
| scan + 最終確認の wall time | 26.341 s | 66.691 s |
| 全 map 解放後の live heap | 152 B | 152 B |

最終確認後の保持量は99.95%、peak live heapは99.62%減った。旧処理も最終的には解放するため、この経路で立証したのは長い lifetime と大きなピークである。累積割当要求や、GUI全体の恒常的なリークと混同しない。

新処理は最終確認で group file を再読込し、ハッシュ計算も行う。この比較では wall time が長くなった。host の他負荷と OS file cache を固定していないため、時間差を厳密な性能比とはしない。通常の unchanged group は既存 snapshot により比較自体を省略する。この比較は source map だけを対象とし、後述の full-route 再構築とは区別する。

[一時計測器](allocation/probes.zip) は System allocator へそのまま委譲し、成功した割当・解放・再割当の byte 数を集計する。サンプルを先に採取し、最後にJSON化した。旧処理は旧 map 保持と2回の clone を再現し、新処理は修正後の実 `group_additions` / `ScannedGroup` / `record_scanned_groups` を呼ぶ。store は保存しない。一時計測用 include と allocator は配信対象ソースから撤去した。

## GUI の短時間比較

変更前 GUI を、この checkout と fresh HOME、実 Chromium の headed dark/light で起動した。12個の custom command Agent PTY は各1秒に512文字を出力し、プロセス開始時刻と PID を照合した。各テーマの全12起動後に60秒観測した。

| テーマ | 観測最大 WS | console/page error |
| --- | ---: | ---: |
| dark | 494,571,520 B | 0 |
| light | 701,833,216 B | 0 |

[dark](baseline/runtime-memory-dark.json)、[light](baseline/runtime-memory-light.json)、[allocator](baseline/allocator.jsonl)、[終了処理](baseline/shutdown.json)、[バイナリ識別](baseline/metadata.json)。起動を含む全体は約9分で、GUI の初回履歴取込中の比較である。4時間の頭打ちは示さない。baseline metadata の source hash は起動時ファイルの記録で、ビルド後に並行実装が始まったためビルド provenance と区別している。baseline の製品処理は `2744c7b6c846abbdcdcdf0595163083e703e8155` に基づき、計測用 allocator と cfg(test) の追加だけを含めてビルドした。

## 初回の長時間試験で判明した残存ピーク

source map だけを削減した修正版を、同じ12 PTY・fresh HOME・実履歴を読む条件で4時間観測する予定だった。しかし dark は7,539.727秒で WS が1,728,106,496 Bとなり、1,500,000,000 Bの上限を超えて失敗した。続く light も2,498,170,880 Bで失敗した。console/page errorは両テーマとも0で、終了時には所有 GUI と producer の停止を確認した。

この GUI 試験では稼働中のリポジトリ履歴を読む。開始時の worktree は497個で、上記の固定 store 割当比較とは入力条件が異なる。並行作業による履歴追加と host 負荷も固定していない。

[dark](cold-failed/runtime-memory-dark.json)、[light](cold-failed/runtime-memory-light.json)、[割当](cold-failed/allocator.jsonl)、[処理段階](cold-failed/slow-phases.json)、[終了処理](cold-failed/shutdown.json)、[バイナリ識別](cold-failed/metadata.json) を失敗のまま保存した。この試験を4時間完了や予算内の証拠には使わない。

初回 worktree scan は7,724.314秒かかり、終了直前の live heap は1,077,421,311 Bだった。続く ref scan は37.595秒で、その終盤には1,666,816,527 B、再構築準備中には約2.84GBとなった。全 source の pending metadata、全 ref の比較 map、source ごとに複製した同じ診断用 container、再構築に不要な別入力 vector が同時に残る経路を確認した。

全面的な streaming 化との比較から、同じ2ファイルでの所有権と解放時期の修正を選んだ。RefBatch の比較結果にも固定サイズの digest を使い、診断用 container は worktree/ref ごとの immutable Arc を共有する。再構築では再走査と復旧検証が成功した直後に元の pending 入力を解放し、incremental 用 vector は必要な経路だけで作る。既存 constructor、fingerprint bytes、永続形式、event ordering、late-shard 発見、破損時の fail-closed 復旧を維持する。Git 側の全 ref descriptor 一覧と全体再構築自体は残るため、constant-memory streaming を実現したとは主張しない。

## 修正前の増加率

再開後の最初の試験は、dark の最初の pane が作成された後、初回の PTY 出力を30秒以内に確認できず、測定開始前に失敗した。light は12 PTY・60秒を完了したが、この試行全体は FAIL として扱う。[dark](setup-failed/runtime-memory-dark.json)、[light](setup-failed/runtime-memory-light.json)、[終了処理](setup-failed/shutdown.json)、[ソースとバイナリ識別](setup-failed/metadata.json)。初回出力だけは既存の launch wizard と同じ90秒の起動予算で待つよう fixture を修正した。定常時の30秒、測定時間、予算値は変えていない。

MB は 1,000,000 bytes とし、増加率は `(最後の値 - 最初の値) / 観測秒数 * 3600 / 1,000,000` で求めた。起動時間や異なるテーマを含む短い区間の時給換算は、定常状態の推定値ではない。

| 区間 | 観測秒数 | WS 始点 → 終点 / 最大 MB | WS MB/時 | private MB/時 |
| --- | ---: | ---: | ---: | ---: |
| 変更前 GUI、dark/light の全サンプル | 502.406 | 138.924 → 701.833 / 701.833 | +4,033.535 | +4,756.073 |
| source map のみ削減した初回試験、dark（失敗） | 7,710.820 | 140.501 → 1,728.106 / 1,728.106 | +741.216 | +792.670 |
| 同じ失敗試験の light | 150.593 | 1,700.405 → 2,498.171 / 2,498.171 | +19,071.013 | +24,640.633 |
| installed v9.123.2、全3時間 | 10,800.748 | 3,091.366 → 1,629.565 / 3,448.828 | -487.233 | -591.027 |
| 同 installed、初回から最大まで | 9,000.661 | 3,091.366 → 3,448.828 / 3,448.828 | +142.974 | +129.413 |

installed の[30分ごとのサンプル](before-installed/process-samples.jsonl)は、2026-10-15T21:48:50.8738878Z から翌00:48:51.6214571Zまでの同じ PID 36240 を記録している。最後の30分では WS が約1,819MB減少したため、全期間の負の増加率を「修正済み」や「1.5GB以下の頭打ち」の証拠にしない。短時間の変更前 GUI と長時間の初回失敗試験も入力・観測期間が異なり、速度比は主張しない。

## 未証明の条件と配信範囲

前回起動の修正版試験も、dark の観測約354.6秒、最後の WS 420,474,880 Bで中断している。[結果](interrupted/runtime-memory-dark.json)、[ソース識別](interrupted/metadata.json)、[中断状態](interrupted/status.json)。全12 producer の起動は確認できたが、4時間完了・light・終了処理の証跡がなく、PASS として扱わない。GUI 再起動後に再実行する。中断の直接原因はこの記録だけでは確定していない。

再起動後の最初の試行では、PowerShell wrapper の任意引数転送が末尾に空文字を加え、Playwright が対象2件でなく全642件を選ぶことを実 command line で確認した。所有 runner のみ停止し、fixture の finally による GUI 停止を確認した。[状態](argument-failed/status.json)、[終了処理](argument-failed/shutdown.json)、[識別](argument-failed/metadata.json)。両 wrapper で空引数を除外し、同じ起動コマンドが2件だけを選ぶことを再確認した。この試行はメモリ観測に含めない。

実モデル Agent 10〜12窓・同条件4時間で WS が1.5GB以下で頭打ちになる Issue #5561 AC-2 は、deterministic PTY で代用しない。provider inference、provider hooks、実装ビルド、PM/Issue Monitor の稼働はこの fixture の対象外である。1.5GB は1,500,000,000 bytesとして判定する。

所有 Issue は未証明の条件を残して OPEN を維持する。配信スライスは source-map の保持量削減と、その回帰・計測証跡である。schema migration は不要で、rollback はこの変更の revert。実 fleet の追加観測は既存 Issue #5561 で追跡する。

User Verification Result: n/a (autonomous)
