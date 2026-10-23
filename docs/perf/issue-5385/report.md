# Issue #5385: 実 agent fleet の GUI リソース観測

## 最終結果

2026-10-15T06:45:46.6757267Z 〜 2026-10-15T07:45:46.6677632Z（UTC）、実経過 3599.987207 秒、61 サンプル。**CPU 予算は未達、元の 7〜8 窓という条件は未証明のため Issue #5385 は open のまま残す。**

| 指標 | 観測結果 | 要求 |
| --- | --- | --- |
| GUI の RSS 最大 | 965,627,904 bytes（0.899311 GiB） | 2 GiB 以下 |
| GUI の平均 CPU | 1.934417 コア | 1 コア未満 |
| implementation 窓数 | 8〜11 | 全期間 7〜8 |

- AC-1: 未証明。採取した RSS は予算内だが、途中で窓数が要求範囲を超えた。
- AC-2: 観測期間の平均 CPU は 1.934417 コアで未達。7〜8 窓に固定した 60 分の平均は、別の再測定が必要。
- AC-3: 超過経路を記録し、PM が Issue #5489 を登録。別経路の Issue #5503 にも追加証拠を渡した。

最初の 8 窓を維持した minute 0〜32 の平均 CPU も 1.781866 コアだった。後半の窓数増加を、前半の CPU 超過の原因とは扱わない。修正を含む版の GUI と 7〜8 窓を維持できる条件で、受け入れを再測定する。

| 窓数の変化点 | UTC | implementation 窓数 |
| --- | --- | --- |
| minute 0 | 2026-10-15T06:45:46.6757267Z | 8 |
| minute 33 | 2026-10-15T07:18:46.6843294Z | 9 |
| minute 38 | 2026-10-15T07:23:46.6865649Z | 11 |
| minute 48 | 2026-10-15T07:33:46.6802091Z | 10 |
| minute 54 | 2026-10-15T07:39:46.6695163Z | 11 |
| minute 55 | 2026-10-15T07:40:46.6778868Z | 10 |

修正済みの配布版 v9.121.1 を、普段使用する Windows HOME と開始時 8 窓の実 Codex implementation agent で観測した一次記録。PM は別枠で稼働し、GUI には gwt を含む 5 プロジェクトが開かれていた。synthetic PTY fixture の結果は含めない。

## 対象と依存修正

開始時に fetch した `origin/develop` と作業ブランチはいずれも `8418ac1512385a8e6745faee12d972f314e5e933`。未配信のソース差分はなかったため、今回の配信対象は不足していた実測証拠である。Spec Status: ALIGNED（plain Issue の実測タスク）。

GUI は PID `32708`、起動時刻 `2026-10-15T06:33:37.3790587Z`。PE の製品バージョンは `9.121.1`、実行ファイル SHA-256 と Windows の版は [context.json](context.json) に収録した。pane API の LISTEN PID も同じ `32708` である。GUI 配下に実在する `codex.exe` 8 プロセスの PID・作成時刻も context に保存した。

`v9.121.1` の commit は `ed1f5d7feb4c0a7ba80cf0a72e6a990e890fa4db`。以下の主修正が release と開始時 develop の両方の祖先であることを `git merge-base --is-ancestor` で確認した。

| 依存 | 主修正 commit | 配送 PR |
| --- | --- | --- |
| Issue #5381: Work ingest のメモリ | `8aafdcc8c` | [PR #5407](https://github.com/akiojin/gwt/pull/5407) |
| Issue #5383: dead namespace の ledger decode 抑止 | `f7bed6d09` | [PR #5405](https://github.com/akiojin/gwt/pull/5405) |
| Issue #5384: 安全な runtime namespace GC | `d75e42b32` | [PR #5409](https://github.com/akiojin/gwt/pull/5409) |

## 採取方法

[collect.ps1](collect.ps1) を `pwsh -NoProfile -File docs/perf/issue-5385/collect.ps1 -GuiProcessId 32708` で実行した。GUI や agent の設定・起動・停止は行わず、既存の GUI プロセスを読み取った。再実行する場合は collector を空のディレクトリへコピーしてから、対象の実 GUI PID を指定する。既存の `samples.jsonl` は上書きしない。

- 開始時と以後 1 分ごとに、合計 61 点を追記する。実経過時間は単調増加する `Stopwatch` で測り、UTC 時刻も併記する。
- GUI の PID と開始時刻を固定し、各回で同一プロセスであることを確認する。
- RSS は Windows `Process.WorkingSet64`。agent・WebView・その他の子プロセスは合計しない。1 分サンプルの最大値を 2 GiB（2,147,483,648 bytes）以下という予算と比較する。
- CPU は GUI の累積 `TotalProcessorTime.TotalSeconds`。平均コア数は `(最終 CPU 秒 − 初回 CPU 秒) / 実経過秒`。論理 CPU 数で割らない。
- 毎回同じ GUI の `pane.list` を保存し、`running` かつ `role=implementation_agent` の実窓数が 7〜8 であることを検査する。窓が開いていることを測定し、全窓が全時間帯でコマンドを実行していたとは主張しない。

原記録は [samples.jsonl](samples.jsonl)。[verify.mjs](verify.mjs) はサンプル不足、プロセスの交代、採取間隔の逸脱、CPU counter の reset、UTC と単調時刻の不一致を拒否する。Windows の起床ジッターは実値で残し、60 分は実経過との差 1 秒未満、各 1 分間隔は 55〜65 秒を要求する。途中の観測を渡したときに、サンプル不足で拒否されることも確認した。

窓数は元の 7〜8 という範囲を維持して判定する。観測途中に新たな実装 agent の起動で 9 窓以上となったため、`fleet_within_requested_range` は false、**AC-1 は厳密な窓数条件では未証明**である。採取した RSS が予算内でも、この逸脱を隠して AC の成功にしない。検査コマンドは失敗条件を結果に明記し Issue を open に残すことを要求する。

## CPU の追加調査

`2026-10-15T06:48:03Z` 頃、同じ配布版 GUI の OS thread CPU counter を約 15.1 秒間隔で比較した。[threads.json](threads.json) に生の差分を収録した。

| thread | CPU 秒 | 区間のコア相当 |
| --- | --- | --- |
| `tokio-rt-worker` / TID 38112 | 8.84375 | 約 0.586 |
| `gwt-monitor-runtime` / TID 36776 | 8.0625 | 約 0.534 |
| `gwt-agent-capacity` / TID 29756 | 1.625 | 約 0.108 |
| `gwt-runtime-daemon-publish-worker` / TID 37744 | 1.21875 | 約 0.081 |

名前付き Monitor thread の経路は `monitor_duplicate_runtime::spawn` → `reconcile_projects` → `reconcile_project` → `session_inventory::observe_sessions`。5 秒の sleep の後に各 repository を処理し、各 census が global runtime sidecar と OS process inventory を読み直す。開いていた root は `E:/gwt`、`E:/160-Idina`、`E:/KeibaSuite`、`E:/unity-cli`、`E:/llmlb`。調査時の runtime は 678 namespaces / 4,036 files / 1,909,602 bytes だった。

[census-probe.rs](census-probe.rs) は既存 checkout library の同じ読み取り関数を 2 pass 呼ぶ診断用プログラムであり、製品の build target ではない。[census-probe.txt](census-probe.txt) に実出力を保存した。**このプローブは dev profile の unoptimized library にリンクしているため、約 0.5 CPU 秒/root という数値を配布版 GUI の CPU 内訳へ換算しない。** 配布版の一次計測は上の OS thread counter と 60 分の GUI process counter である。tokio thread の仕事や CPU 超過の全量を census に帰属させる証拠はまだない。

Issue #5383 の dead ledger decode 抑止と Issue #5384 の GC が入っていても、複数 root による census 反復経路のコストは残っている。Issue #5404 が担当する Work projection の clone・broadcast とは別の経路として PM に報告し、後続 [Issue #5489](https://github.com/akiojin/gwt/issues/5489) に登録された。

既存の `perf_regression_test::session_census_with_six_thousand_dead_namespaces_stays_within_read_budget` は `runtime_tests` target で実行し PASS。最初の実行は 6,000 dead namespaces に対して 1.321 秒、累積 allocation 19,422,120 bytes だった（異常値 gate は 5 秒 / 50 MB、1 秒は目標値）。この synthetic fixture の成功は実 fleet の CPU 予算達成を示さない。

### Usage poller の別経路

独立した読み取りレビューでは、`usage_poller::run` → `Poller::poll_once` → `collect_sessions` が 30 秒ごとに active status の ledger 候補を列挙し、host/child の生存確認をせずに transcript を全文取得することを確認した。Codex は `read_codex_session` → `rollout_for_session` で Session ごとに rollout ツリーを再走査し、`last_token_count` と `last_model` で全文を二度解析する。account 側も直近 8 ファイルを全文読み直す。この経路の既存 owner は [Issue #5503](https://github.com/akiojin/gwt/issues/5503)。

07:19:52〜07:20:58 UTC のファイル metadata と ledger field の集計は次のとおり。prompt・transcript の本文・credential は出力せず、HTTP/API 呼び出しも行っていない。

集計には Python 3.14 の `os.walk` / `Path.stat` / `tomllib` を使い、ledger の ID と filename stem の一致、status、provider、provider Session ID の有無、mtime だけを数えた。rollout は filename / mtime / size、Claude は `projects/*/<session-id>.jsonl` の存在と size を照合した。生存 PID の一覧だけ `tasklist /FO CSV /NH` から取得した。

| 対象 | 件数 | ファイルサイズの合計 |
| --- | --- | --- |
| Codex rollout 全体 | 1,930 | 8,368,040,156 bytes |
| Codex account の直近 rollout | 8 | 6,059,042 bytes |
| Codex active 候補に名前が一致する rollout | 157 | 618,719,187 bytes |
| Claude active 候補に名前が一致する transcript | 164 | 915,047,414 bytes |

ledger の active status 候補は 1,103 件で、provider Session ID を持つ Codex は 161 件、Claude は 245 件だった。Codex の実装上の metadata 訪問候補は `(161 + account 1) × 1,930 = 312,660 回/tick`、全文読み取り対象候補は Codex と Claude の合計約 1.534 GB/tick。いずれもファイル名と size による候補量であり、実際の read/parse 回数や GUI CPU 秒を計測した値ではない。同じ transcript を指す ledger 行も consumer と同じく重複計上した。

active 候補のうち 1,061 件は生存する PID namespace に対応する sidecar がなく、1,052 件は mtime が 24 時間超だった。これは metadata の指標であり、exact child identity による死亡判定ではない。また `SessionLedgerCache::new()` は共有 loader を利用するため、「毎回すべての TOML を parse する」とは説明しない。`tokio-rt-worker` は blocking pool にも使われる名前であり、usage poller への確定帰属には TID と thread CPU を持つ追加計測が必要である。

## 配信の検証範囲

Acceptance Surface: non-user-facing（既存 GUI プロセスの資源計測・記録のみ。画面、テーマ、入力操作、製品ロジックの変更はない）。既存 GUI を実運用中に観測した記録であり、隔離 fixture に置き換えてはいない。

実測の最終集計は [result.json](result.json) に保存し、`node docs/perf/issue-5385/verify.mjs` で原記録との一致を再計算する。このコマンドの成功は証拠の完全性・集計の一致を意味し、失敗した性能予算を成功へ変更しない。予算未達の場合は登録済みの後続 Issue が必要である。

Launch mode: autonomous

User Verification Result: n/a (autonomous)

Agent Visual Check: n/a (no UI surface)
