# Issue #5561: Work ingest の source map 保持量

## 対象と原因

installed v9.123.2 の GUI（PID 36240、2026-10-15T14:11:46Z 起動）で private memory の増加を観測した。読み取り専用の private-region サンプルでは、約865MBの検査範囲に `.jsonl` が984,543回、`.gwt` が1,070,144回現れた。[サンプル](before-installed/private-region-sample.json) は内容を出力せず、固定マーカーと領域サイズを集計したもの。解放済みの heap page も含むため、これだけを live leak の証明には使わない。

実データは495 worktree、1,232 fingerprint group、2,430,572 source、group file 約484MBだった。[規模](before-installed/source-counts.json)。変更前の Work ingest は、変更グループの全 source map と、比較のために読み込んだ旧 map を処理終了まで保持し、最終確認でさらに旧 map を2回複製していた。

製品コードの変更は Work ingest、intake、Git blob batch の既存3ファイルを対象とする。まず `ScannedGroup` を件数と length-delimited SHA-256 に変え、比較済みの正常な旧 map を解放した。dirty な移行データと読込失敗は保持し、最終確認は追加なしで複製0回、追加ありで1回とした。

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

公開用の runtime JSON は、パス中のアカウント成分だけを秘匿化している。計測値・原本の割当ログ・再現入力は変更していない。

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

再開後の4時間試験も2回失敗した。最初の dark は観測約54分で定常出力の30秒predicate待機がタイムアウトした。最大WSは601,468,928 B、browser errorは0、12 producerは存続していた。[結果](trace-failed/runtime-memory-dark.json)、[traceのエラー要約](trace-failed/failure.json)、[識別](trace-failed/metadata.json)、[終了処理](trace-failed/shutdown.json)。失敗後のtrace保存は約7.6GBのresourceを処理しており、その間のGUI生存やWS減少は観測成功に数えない。所有runnerを停止し、wrapperのGUI/producer cleanup完了を確認した。巨大なraw traceはリポジトリに含めない。

連続traceを無効にした次の dark は観測1,634.624秒まで進んだが、producerのPID/開始時刻を照合する `execFileSync("pwsh")` が30秒でタイムアウトした。最大WSは610,729,984 Bで、予算超過ではない。lightは60秒を完了したが試行全体はFAILである。[dark](sampler-failed/runtime-memory-dark.json)、[light](sampler-failed/runtime-memory-light.json)、[識別](sampler-failed/metadata.json)、[終了処理](sampler-failed/shutdown.json)。このfixtureは各loopで新しいPowerShellを2回起動していた。繰り返しのcold process起動を避ける常駐samplerへ変更し、同じPIDと開始時刻の照合、応答期限、観測時間、予算のassertionを維持して再試験する。

常駐samplerでは起動timeoutを越えて観測できたが、11,982.794秒でWSが1,556,852,736 Bとなり、同じ1.5GB予算で失敗した。直前7秒のWSは1,174,196,224 Bで、増加段差を記録した。browser errorは0。rootのintake index保存は未確認である。[dark](cold-shared-failed/runtime-memory-dark.json)、[light](cold-shared-failed/runtime-memory-light.json)、[識別](cold-shared-failed/metadata.json)、[終了処理](cold-shared-failed/shutdown.json)。この試行も4時間完了・頭打ちの証拠にはしない。

再構築loaderは元のpending入力を保持しながら、返却するkey/fingerprintの文字列を全件複製していた。さらに同じpayloadを多数含むexact-size iteratorをHashMapへcollectし、unique件数より大きなbucket容量を予約する。loader完了後のpending解放では、この途中の重複を防げない。loaderが入力Vecを消費し、ref metadataをmove、local metadataを所有mapから取り出す方式へ変更する。再走査時のmetadataを正本にし、破損復旧で必要な期待値だけは従来どおり保持・照合する。payload cacheはunique contentだけを順に追加する。

loaderだけの追加比較では、同一payloadを参照する20万件の合成ref sourceを使用した。実loaderの関数本文を抜き出し、空のworktree経路は未到達stub、共有payloadのdigestは同じ64 bytesとしてSystem allocatorへの成功要求を集計した。GUI、decode/fold、永続化、実履歴全体は含まない。[入力・限界](loader-allocation/metadata.json)、[旧](loader-allocation/old.jsonl)、[新](loader-allocation/new.jsonl)、[再実行可能なRustソース](loader-allocation/probes.zip)。

| loaderの計測値 | 旧借用方式 | 所有権移譲後 |
| --- | ---: | ---: |
| 入力構築後のlive heap | 65,600,056 B | 65,600,056 B |
| peak live heap | 127,149,128 B | 80,001,388 B |
| payload cache件数 / capacity | 1 / 229,376 | 1 / 3 |
| 返却後のlive heap | 50,801,080 B | 64,001,080 B |
| 全入力・出力解放後のlive heap | 1,024 B | 1,024 B |

peakは約37.08%減った。返却後の値は、元のString capacityを移譲するため増えている。旧方式のcloneはcapacityを文字列長へ縮めていたが、loader途中では元と複製が併存する。ここで改善したのは途中ピークであり、あらゆる段階の保持量が下がるという比較ではない。最終1,024 Bは標準出力のbufferを含む。実GUIの約380MB段差すべてをこの小さい合成比較に帰属させない。

loader の所有権移譲を含む `815fcff09d93fb3e88229445cf2239ed5bf98369` を正確にビルドした次の試行も、dark の観測13,182.808秒で WS が1,559,937,024 Bとなり失敗した。直前の WS 約1.17GBから増える同じ段差が残った。light も1,572,171,776 Bで失敗した。browser error は両テーマ0、所有GUIとproducerのcleanupは完了した。[dark](owned-loader-failed/runtime-memory-dark.json)、[light](owned-loader-failed/runtime-memory-light.json)、[識別](owned-loader-failed/metadata.json)、[外部45秒サンプル](owned-loader-failed/monitor.jsonl)、[終了処理](owned-loader-failed/shutdown.json)。dark の最初から最後までの WS は117.223→1,559.937 MB、+393.980 MB/時、private は+406.527 MB/時で、処理中の増加を含む失敗試験の値である。4時間完了や定常状態の傾きとはしない。root intake index 保存前に失敗しており、loader単体の改善からGUI全体の改善を推定できない。

次に実Git履歴の batch API を単独で計測した。749 refs、589 unique commits、873,959 matrix descriptors、8,514 unique payload OIDsで、descriptor cache完成時のlive heap 194,950,932 Bが、input-order matrix複製後350,509,140 B、payload読込後407,967,244 B、返却値複製後594,173,124 Bとなった。約399MBの段階増加はGUIの段差と量的に整合するが、同じGUI内のallocation stackを採取した結果ではない。公開APIと完全matrix selectorを維持し、同じcommitの最後の入力出現でVecを移譲し、返却時もpath/OIDのStringをmoveする最小修正を選んだ。全refのchunk分割はOID一度readの契約に影響し、公開fieldをArcへ変える案は不要なAPI変更になるため採用しない。

この749 refsのcommit一覧を固定し、旧・新を同じ入力readerで取り直した。[入力](git-batch-allocation/frozen-input.json)、[旧](git-batch-allocation/old.jsonl)、[新](git-batch-allocation/new.jsonl)、[比較](git-batch-allocation/comparison.json)、[再現ソース](git-batch-allocation/probes.zip)、[実行説明](git-batch-allocation/README.md)。計測器は実 `blob.rs` のコピーへphase captureを加え、System allocatorへ委譲した成功要求を集計する。selectorは全OIDを選択し、callerのfingerprint構築、worktree scan、fold、永続化、GUI RSSは含まない。

| Git batch の計測値 | 旧複製方式 | 所有権移譲後 |
| --- | ---: | ---: |
| descriptor cache完成時のlive heap | 194,807,198 B | 194,832,814 B |
| input-order matrix完成時 | 350,365,406 B | 214,140,176 B |
| payload読込後 | 407,823,510 B | 271,598,280 B |
| output完成時 | 594,029,390 B | 290,338,712 B |
| peak live heap | 594,029,390 B | 290,731,928 B |
| API返却後 | 242,069,839 B | 242,063,239 B |
| 累積割当要求 | 1,036,785,871 B | 786,944,945 B |
| API wall time | 10.018 s | 9.571 s |
| output解放後 | 168,351 B | 168,351 B |

peakは約51.06%減り、返却後の保持量はほぼ同じだった。必要な重複入力alias分のdescriptor複製は残す。raw tree cacheへの追加変更は、この2点のmoveで十分な削減が得られたため行わない。実行順とhost負荷を固定しておらず、約0.45秒の時間差を速度改善とはしない。この測定もGUI全体の4時間予算の代わりにはしない。

所有 Issue は未証明の条件を残して OPEN を維持する。配信スライスは source metadata の保持量と複製ピークの削減、およびその回帰・計測証跡である。schema migration は不要で、rollback はこの変更の revert。実 fleet の追加観測は既存 Issue #5561 で追跡する。

2026-10-19 の再開時に fetch した `origin/develop` (`f16d1243095672d50fdd7e4b4681d0832ce68a6c`) は、修正3コミット `109b7c5b7` / `815fcff09` / `5767b7cfa` を含んでいない。未配信ソースは AC 達成の証拠とせず、まず保持量削減の独立配信と互換性を検証する。AC-1 は上記の範囲付き allocation 計測、AC-3 は区間・単位・失敗を明記した増加率を証拠とする。AC-2 は実モデル Agent 10〜12窓・同条件4時間での観測が引き続き必要で、短時間の headed 回帰が成功してもチェックしない。

## v9.129.4 の実運用で残った増加

2026-10-20 の再開時、PR #5742 は `origin/develop` に包含済みで、元の3つの修正コミットに未配信分はなかった。稼働 GUI はそれを含む installed v9.129.4、PID 13508、00:34:05Z 起動だった。03:10:59Z の読み取りでは WS 2,276,139,008 B、private 2,301,476,864 B と、既に1.5GBの予算を超えていた。[プロセス識別と限界](release-9.129.4-failed/metadata.json)、[サンプル](release-9.129.4-failed/process-samples.jsonl)。12 Codex と1 PMの13窓、provider process は14本で、制御された10〜12窓の比較ではない。この失敗を AC-2 の成功や修正後の増加率には使わない。

共有 perf stream には、worktree_scan 5,784,699 ms、続く ref_scan 8,634 ms と、その後の intake 未完了が記録されていた。[区間](release-9.129.4-failed/ingest-phases.json)。このログには PID がなく、稼働 GUI への帰属は推論である。非停止の native stack でも worker のファイル操作を観測したため、loader の再走査も候補に残す。private page の固定マーカー集計は解放済み領域を含み、live heap の証明には使わない。[集計](release-9.129.4-failed/private-region-sample.json)。最後に保存された advisory index は1,291 groups / 2,926,491 sources / group file 584,186,244 Bだったが、現在の走査入力や生存量ではない。[index](release-9.129.4-failed/intake-index-state.json)。

local source 20万件だけの loader 抜粋比較でも、入力 live 73,600,056 B に対して peak 113,560,396 B、全 source の remove 後・返却直前 69,840,700 B、返却後52,801,080 Bを観測した。[結果](release-9.129.4-failed/local-loader-before.jsonl)。空になった prelock HashMap の bucket は返却まで残る。この合成経路はファイル読取・検証・実 container を stub にしており、GUI の2GB全体を説明しない。再現ソースは後述の archive に含めた。

## collector の decoded cache と重複割当

既存 collector は unique content ごとの decoded WorkEvent 全件を返却直前まで保持し、source の alias ごとに event を clone・JSON 化してから2件上限を判定していた。返却時の列挙順を変える accumulator 案や全経路の streaming 化と比較し、先頭出現順の content identity 集約と multiplicity を選んだ。raw Arc は identity の有効期間を保ち、各 line は1度 decode・serializeし、採用する event を既存の返却 Vec へ moveする。完全同一イベントの canonical と provenance 用 duplicate の最大2件、全 report 件数、source container の診断専用扱いと破損復旧条件は維持する。既存 contextual-content test に allocation 移譲の assertion を加え、修正前の失敗を確認した。[RED](release-9.129.4-failed/collector-red.log)。

実 WorkEvent と decoder をリンクした合成入力で、旧本文と修正後の製品関数を抜粋して比較した。変更は関数名と3つの計測器だけで、復元した候補関数の hash は製品と一致する。[結果・関数 hash](collector-allocation/collector-product.json)、[再実行手順と限界](collector-allocation/collector-reproduce.md)、[全 source](collector-allocation/probes.zip)。

| 同一入力 | 旧 peak live | 修正後 peak live | 旧累積割当要求 | 修正後累積割当要求 |
| --- | ---: | ---: | ---: | ---: |
| 64 unique × 4 events × 701 aliases、title 1,024 B | 4,262,061 B | 2,108,784 B | 1,061,810,568 B | 3,923,640 B |
| 2,000 unique × 4 events × 8 aliases、title 2,048 B | 150,055,181 B | 115,225,280 B | 787,427,448 B | 188,809,304 B |

後者の peak は約23.21%、累積割当要求は約76.02%減った。両入力とも完全 stable key のソート済み digest、key総bytes、decode・返却・duplicate件数が一致した。close/opaque/invalidやcontainer警告の意味は製品回帰で検証し、この合成入力の等価性だけで一般化しない。測定値は System allocator への成功要求で、GUI WS/private、再走査、fold、永続化、実 Agent の4時間運用を含まない。AC-2 と同条件の前後 MB/時は引き続き未証明である。

User Verification Result: n/a (autonomous)
