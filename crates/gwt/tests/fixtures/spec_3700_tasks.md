## Tasks

凡例: `[P]` = 他の `[P]` タスクと並列実行可能（ファイル競合なし）。各タスクの末尾に対応 FR を記す。
**テストファースト厳守**: 各フェーズの `T*-RED` を先に書き、RED を確認してから同フェーズの実装タスクに入る。

新規ファイルは「新規」と明記。それ以外は develop に実在を確認済みのパス。

---

### Phase 0: 設定と kill switch（最初に入れる。後付け禁止）

- [x] **T001-RED** `crates/gwt-config/src/settings.rs` の `#[cfg(test)]` に、`Settings` のデフォルトが perf 設定を持ち、既定で「有効・保持 30 日・自己予算 1%」であること、および `~/.gwt/config.toml` の perf セクションで各値を上書きできることを検査するテストを追加する。設定欠落時に既存 TOML が壊れないこと（`#[serde(default)]` 互換）も検査する。— FR-004 / FR-005 / FR-009
- [x] **T002** `crates/gwt-config/src/settings.rs` に `PerfConfig` を定義し、`Settings` に `perf` フィールドを 1 つ追加する。フィールドは「有効フラグ（kill switch）」「保持日数」「自己予算 CPU 割合」「budget 上書き（省略時はコード内定数）」。既存 `profiling: bool` は意味が異なるため流用しない。— FR-004 / FR-005 / FR-009

---

### Phase 1: perf レコードモデル・JSONL 保存・ローテーション

- [x] **T010-RED** `[P]` 新規 `crates/gwt/src/perf/record.rs` の `#[cfg(test)]` に、perf レコード（sample / violation）のシリアライズ契約テストを書く: `schema_version` が先頭にあること、系統タグが `ui` / `op` / `resource` の 3 値であること、`type` で sample と violation が判別できること。— FR-004
- [x] **T011-RED** `[P]` 同上ファイルに **sanitize 回帰テスト**を書く: `data_base64` / `payload` / `input` / `text` などのブロック対象フィールドが出力に現れないこと、ローカル絶対パス文字列が対象名に混入した場合に落ちること、160 文字超の文字列が切り詰められること。`crates/gwt/src/app_runtime/ui_trace.rs` の既存テスト `save_ui_trace_to_log_dir_writes_jsonl_artifact`（`must-not-leak` を検査する形）を踏襲する。— FR-010
- [x] **T012-RED** `[P]` `crates/gwt-core/src/logging/housekeep.rs` の `#[cfg(test)]` に、`housekeep_at` が任意の prefix（`perf-`）と日付フォーマットの日次ファイルを保持期間で削除し、期間内を残すテストを追加する。既存の `gwt.log.YYYY-MM-DD` に対する 2 テスト（`retention_zero_disables_cleanup` / `deletes_only_files_older_than_retention`）が引き続き GREEN であることも同時に守る。— FR-004
- [x] **T013** `crates/gwt-core/src/logging/housekeep.rs` の `housekeep_at` を、ファイル名 prefix と日付サフィックス形式を引数で受け取れる形に一般化する。既存呼び出し元 `crates/gwt-core/src/logging/init.rs:101` は挙動不変のまま引数を明示する形に更新する。— FR-004
- [x] **T014** 新規 `crates/gwt/src/perf/record.rs` に perf レコード型と sanitize 適用を実装する。sanitize は `crates/gwt/src/app_runtime/ui_trace.rs` のブロックフィールド定義と `crates/gwt/src/app_runtime/frontend_action_log.rs` の 160 文字キャップ / 制御文字除去を参照して再実装しない形にする（必要なら両ファイルから `pub(crate)` へ最小昇格）。— FR-004 / FR-010
- [x] **T015-RED** 新規 `crates/gwt/src/perf/store.rs` の `#[cfg(test)]` に、日次ファイル `perf-YYYY-MM-DD.jsonl` への追記、日付跨ぎでのファイル切り替え、起動時ハウスキープの呼び出しを検査するテストを書く。時計は注入可能にし、実時間に依存しない。`gwt_core::test_support::ScopedGwtHome` で HOME を temp に固定する。— FR-004
- [x] **T016** `crates/gwt/src/perf/store.rs` を実装する。保存先は `crates/gwt-core/src/paths.rs:815` の `gwt_logs_dir()` に `perf` を足したディレクトリ。常駐ローテーションタイマーは持たず、書き込み時の日付比較のみで切り替える。— FR-004
- [x] **T017** 新規 `crates/gwt/src/perf/mod.rs` を作り、`crates/gwt/src/lib.rs`（または `crates/gwt/src/main.rs` のモジュール宣言）に `perf` モジュールを登録する。kill switch が無効なら全公開 API が最初の分岐で no-op になることをここで保証する。— FR-009

---

### Phase 2: budget 定数と平滑化（violation 判定）

- **T020-RED** `[P]` 新規 `crates/gwt/src/perf/budget.rs` の `#[cfg(test)]` に budget テーブルのテストを書く: UI 操作応答 100ms / 描画フレーム 16ms / gwtd read p95 100ms / mutation p95 500ms が既定で引けること、launch・build 系 operation が別枠（除外または個別上限）として引けること、メモリ閾値が `crates/gwt/src/runtime_health_poller.rs` の `WARN_MEMORY_BYTES` / `HOT_MEMORY_BYTES` と同一値であること（コピーではなく参照であること）。— FR-005
- **T021-RED** `[P]` 同上ファイルに、`crates/gwt/src/cli/json_envelope.rs:184` 以降の match アームに現れる全 operation 名が read / mutation / 別枠のいずれかに分類済みであることを検査するテストを書く（未分類 operation の検出）。— FR-005
- **T022-RED** `[P]` 新規 `crates/gwt/src/perf/smoothing.rs` の `#[cfg(test)]` に平滑化テストを書く: 単発の budget 超過では violation が出ないこと（受け入れシナリオ 7）、N 回連続で初めて violation が 1 本出ること、回復するまで同じ violation を再記録しないこと、T 秒継続でも violation になること。`crates/gwt/src/runtime_health_poller.rs` の `sustained_severity_requires_three_consecutive_hot_samples` と同じ形の決定的テストにする。— FR-006
- **T023** `crates/gwt/src/perf/budget.rs` を実装する。既定はコード内定数、上書きは `PerfConfig`（T002）経由のみ。— FR-005
- **T024** `crates/gwt/src/perf/smoothing.rs` を実装する。対象名ごとの連続超過カウンタと初回超過時刻を持つ。p95 判定系は直近ウィンドウの p95 で評価する。— FR-006
- **T025** `crates/gwt/src/perf/mod.rs` に perf sink（受け取る → 平滑化 → 追記）を実装し、Phase 1 の store と Phase 2 の budget / smoothing を結線する。— FR-004 / FR-006

---

### Phase 3: 収集経路 C（リソース履歴 / 既存 poller 再利用）

- **T030-RED** `crates/gwt/src/runtime_health_poller.rs` の `#[cfg(test)]` に、`poll_once` が生成した既存スナップショットが perf sink に渡ること、**新しい `sysinfo::System` インスタンスも新しい tick も増えていないこと**、`should_broadcast` が false（変化なし）でも perf サンプルは記録されること、kill switch 無効時は sink に何も渡らないことを検査するテストを追加する。既存の `poll_budget_runs_full_reconciliation_every_60_seconds` / `poll_budget_broadcasts_on_change_or_15s_heartbeat` が GREEN のままであることを守る。— FR-003 / FR-009
- **T031** `crates/gwt/src/runtime_health_poller.rs` の `run` / `poll_once` に、既存スナップショットを perf sink へ渡す分岐のみを追加する。ポーリング周期・refresh scope・broadcast 判定には一切触れない。永続化の間引きは sink 側で行う。— FR-003
- **T032** クライアント未接続時（`clients.has_clients()` が false で tick 本体が skip される現行挙動）に perf サンプルが途切れる件の扱いを決めて実装する。**要確認**: 「常時収集」を満たすには GUI 未接続時も記録するか、記録空白を許容して perf ログにギャップとして残すかの選択が要る。既定は「空白を許容し、集計側でギャップを明示する」。— FR-003

---

### Phase 4: 収集経路 B（op 所要時間 / backend + gwtd）

- **T040-RED** `[P]` `crates/gwt/src/app_runtime/tests.rs`（`--bin gwt` ターゲット）に、`handle_frontend_event`（`crates/gwt/src/app_runtime/mod.rs:5152`）が 1 イベント処理につき `op` 系統のサンプルを 1 本だけ記録すること、対象名が `FrontendEvent` の kind 由来識別子であること、kill switch 無効時は記録しないことを検査するテストを追加する。— FR-002 / FR-009
- **T041-RED** `[P]` 新規 `crates/gwt/tests/perf_op_duration_test.rs` に、`crates/gwt/src/cli/json_envelope.rs:47` の `dispatch` が gwtd operation 1 回につき `op` 系統サンプルを 1 本記録し、対象名が operation 文字列であることを検査するテストを書く。— FR-002
- **T042** `crates/gwt/src/app_runtime/mod.rs:5152` の `handle_frontend_event` に開始時刻取得と戻り値直前の 1 サンプル記録を追加する。個別ハンドラには手を入れない。— FR-002
- **T043** `crates/gwt/src/cli/json_envelope.rs:47` の `dispatch` を計測で包む。operation 名は既存 match の文字列をそのまま使う。— FR-002

---

### Phase 5: 収集経路 A（UI 応答 / 描画 / 常時モード）

- **T050-RED** `[P]` `crates/gwt/web/__tests__/ui-trace-profiler.test.mjs` に、常時計測モードのテストを追加する: 手動トレース非 active でも `measure` が duration を perf sink コールバックへ渡すこと、常時モードでは診断用リングバッファ（`DEFAULT_MAX_ENTRIES=2000`）に積まないこと、手動トレースの `start` / `stop` / 保存ペイロード形状が従来どおりであること（回帰）。— FR-001
- **T051-RED** `[P]` `crates/gwt/web/__tests__/ui-trace-wiring.test.mjs` に、`createUiTraceWiring` が常時計測フラグと perf sink を受け取り、`traceMeasure` / `tracePointer` が常時モードで sink に流れることを検査するテストを追加する。— FR-001
- **T052-RED** `[P]` 新規 `crates/gwt/web/__tests__/perf-sample-batching.test.mjs` に、UI サンプルのバッチ送信テストを書く: サンプルは即時送信せずバッチにまとめること、kill switch でバッチャが完全に no-op になること、ブロック対象フィールドがバッチに含まれないこと。— FR-001 / FR-009 / FR-010
- **T053** `crates/gwt/web/ui-trace-profiler.js` に常時計測モードを追加する。既存 `active`（手動トレース）の意味と保存形式は変えない。`raf_gap` プローブの 50ms しきい値とは別に、描画フレーム 16ms budget 用のフレーム時間サンプルを出す。— FR-001
- **T054** `crates/gwt/web/ui-trace-wiring.js` の `createUiTraceWiring` に常時計測フラグと perf sink 引数を追加する。`crates/gwt/web/app.js` の既存 `traceMeasure` 呼び出し（1679 / 2530 / 3386 / 4153 / 5372 行付近）は変更しない。— FR-001
- **T055** 新規 `crates/gwt/web/perf-sample-batcher.js` を作り、UI サンプルのバッチ化と送信を実装する。送信は `crates/gwt/web/app.js:869` の既存 `send()` 経由のみ。— FR-001
- **T056** `crates/gwt/web/app.js`（149 / 195 / 894 / 1192 行付近）に batcher の生成と wiring への注入を配線する。UI 側の追加はこの 1 ファイルに閉じる。— FR-001
- **T057** `crates/gwt/web/socket-receive-dispatcher.js` の既存 `trace()` 経路に、受信→適用の所要時間サンプルを相乗りさせる。新しいループは作らない。— FR-001
- **T058-RED** `crates/gwt/src/protocol.rs` の `#[cfg(test)]` に、UI perf サンプルのバッチを運ぶ `FrontendEvent` バリアントの wire 形状（snake_case、件数上限、スカラー限定）を検査するテストを追加する。既存の `frontend_event_save_ui_trace_deserializes_payload` を踏襲する。— FR-001 / FR-010
- **T059** `crates/gwt/src/protocol.rs` に UI perf サンプルバッチの `FrontendEvent` バリアントを追加し、`crates/gwt/src/app_runtime/mod.rs:5945` 付近の match（`FrontendEvent::SaveUiTrace` の隣）に perf sink へ渡すアームを追加する。— FR-001
- **T060** `scripts/run-frontend-unit-tests.sh` の実行リストに T052 の新規テストファイルを追記する（**リストに載せないと CI で実行されない**）。— FR-001

---

### Phase 6: 自己予算とレート低下

- **T070-RED** `[P]` 新規 `crates/gwt/src/perf/self_budget.rs` の `#[cfg(test)]` に自己予算テストを書く: 計測系の CPU 寄与が既定 1% を超えたら次周期でサンプリングレートが下がること、下がったレートが回復条件で戻ること、kill switch でサンプリングが完全停止すること、レート低下後も violation 判定が破綻しない（サンプル間隔の変化が平滑化条件に織り込まれる）こと。— FR-009
- **T071** `crates/gwt/src/perf/self_budget.rs` を実装する。CPU 寄与の判定入力は `crates/gwt/src/runtime_health_poller.rs` が既に取得している自プロセス CPU を使い、計測のための追加計測を導入しない。— FR-009
- **T072** `crates/gwt/src/perf/mod.rs` の sink に self_budget を結線し、Phase 3 / 4 / 5 の全収集経路が同じレート制御下に入ることを保証する。— FR-009

---

### Phase 7: gwtd perf operation

- **T080-RED** `[P]` 新規 `crates/gwt/src/cli/perf.rs` の `#[cfg(test)]` に、`PerfCommand` の引数パース（期間 / 系統 / 対象フィルタ）と、JSON 出力が必須であること（`crates/gwt/src/cli/diagnostics.rs` が `json: false` を明示エラーにしている前例に合わせる）を検査するテストを書く。— FR-007
- **T081-RED** `[P]` 新規 `crates/gwt/tests/perf_cli_test.rs` に、`perf.summary` が対象ごとに 件数 / p50 / p95 / 最悪値 を返すこと、`perf.violations` が violation レコードのみを返すこと、期間・系統・対象フィルタが効くこと、perf ログ不在時に空結果でエラーにならないことを検査するテストを書く。`crates/gwt/tests/gwtd_cli_test.rs` の既存 envelope テストの形を踏襲する。— FR-007
- **T082-RED** `[P]` 同上ファイルに、**violation から Issue が自動起票されないこと**（gh 呼び出しが一切発生しないこと）を検査するテストを書く。— FR-011
- **T083** `crates/gwt/src/cli/perf.rs` を実装する（`PerfCommand` + `parse` + `run` の 3 点セット、`crates/gwt/src/cli/diagnostics.rs` と同構造）。— FR-007
- **T084** `crates/gwt/src/cli.rs` にモジュール宣言（8〜52 行のモジュール一覧）、`CliCommand` 列挙（165 行〜）への `Perf(PerfCommand)` 追加、`run` 振り分け（739 行付近）へのアーム追加を行う。— FR-007
- **T085** `crates/gwt/src/cli/json_envelope.rs`（184 行〜の operation match）に `perf.summary` / `perf.violations` の 2 アームを追加する。— FR-007

---

### Phase 8: CI perf 回帰テスト

- **T090-RED** `[P]` 新規 `crates/gwt/tests/perf_regression_test.rs` に、**相対基準**の perf 回帰テストを書く: 同一 run 内でベースライン操作と代表操作の所要時間比を測り、比率が閾値を超えたら RED になる。絶対値アサーションは「明らかに異常」の 1 段だけ、実測の桁を大きく上回る位置に置く（#3699 の flake 教訓）。— FR-008
- **T091** `crates/gwt/tests/perf_regression_test.rs` に、劣化注入フック（テスト専用の遅延注入）を実装し、**注入ありで RED・注入なしで GREEN** をデモできる形にする（受け入れシナリオ 4）。— FR-008
- **T092** `[P]` 新規 `crates/gwt/playwright/tests/perf-budget.spec.ts` に、embedded-frontend（実 chromium・backend スタブ）で UI 操作応答と描画フレームの相対基準を検査する spec を追加する。実エージェント起動は不要な形にする。— FR-008 / FR-001
- **T093** `.github/workflows/test.yml` の `test` ジョブに perf 回帰テストのステップを追加する（`timeout-minutes` 明示、既存ステップの流儀に合わせる）。frontend 側は `test-frontend` ジョブの既存 `bash scripts/run-visual-tests.sh` に自動的に含まれることを確認する。— FR-008

---

### Phase 9: 統合と検証

- **T100** `crates/gwt/src/main.rs` の起動シーケンス（8141 行付近の logging 初期化の後）に perf ハウスキープの 1 回実行を配線する。**`crates/gwt/src/main.rs:2411` の `logging_initialization_sources_do_not_use_legacy_log_dir` ガードに抵触しない形にする**（perf の書き出しは `app_runtime/mod.rs` 本文と logging 初期化の外に置く）。— FR-004
- **T101** 全経路の通し確認: gwt を通常起動し、UI 操作・gwtd operation・アイドル放置を行い、`~/.gwt/logs/perf/perf-YYYY-MM-DD.jsonl` に 3 系統すべてのサンプルが出ること、`gwtd` の `perf.summary` / `perf.violations` が結果を返すことを実地で確認する（受け入れシナリオ 1 / 3 / 6）。— FR-001〜FR-007
- **T102** 計測有効の通常稼働で、perf 系プロセス起因の PERF HOT（CPU 100% / メモリ 2GB 超）が発生しないことを Runtime Health ステータスストリップで確認する（受け入れシナリオ 5、成功基準）。— FR-009
- **T103** perf ログ実物を目視し、端末ペイロード・入力内容・ローカル絶対パスが 1 行も含まれないことを確認する（FR-010 の実地確認）。— FR-010

#### 最終検証（全タスク完了後・順に全部実行する）

- **T110** `cargo fmt --all -- --check`
- **T111** `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- **T112** `cargo test -p gwt --lib --no-fail-fast`
- **T113** `cargo test -p gwt --bin gwt --no-fail-fast` — **`app_runtime/` と `runtime_health_poller.rs` のテストはこのターゲットにしか存在しない。省略すると偽 GREEN になる**
- **T114** `cargo test -p gwt-core --lib --no-fail-fast` / `cargo test -p gwt-config --lib --no-fail-fast`
- **T115** `cargo test -p gwt --test perf_op_duration_test --test perf_cli_test --test perf_regression_test --no-fail-fast`
- **T116** `cargo doc --workspace --no-deps --document-private-items`（CI の Clippy & Rustfmt ジョブがこれも実行する）
- **T117** `bash scripts/run-frontend-unit-tests.sh` / `bash scripts/run-frontend-smoke-tests.sh` / `bash scripts/check-frontend-bundle.sh`
- **T118** `bash scripts/run-visual-tests.sh`（Playwright embedded-frontend）
- **T119** markdownlint（変更した Markdown がある場合のみ）

> 検証出力を `| head` / `| grep` に流さないこと（SIGPIPE でテストが途中終了し、検証が無効になる）。失敗が出たら `--no-fail-fast` で全件洗ってから修正に入る。

---

### 依頼元へのメモ（実装前に確定が要る点）

1. **保存先**: SPEC は `~/.gwt/logs/perf/`。現行のログはプロジェクト単位（`~/.gwt/projects/<hash>/logs/`）が正で、`crates/gwt/src/main.rs:2411` に `gwt_logs_dir()` 使用を禁じるガードがある（対象は `main.rs` / `app_runtime/mod.rs` の本文のみ）。本 Plan は SPEC 裁定どおりグローバルで進める前提で書いている。
2. **round trip の範囲**（T042 / FR-002）: `request_id` を持つ `FrontendEvent` は一部のみ。全イベントへの相関 ID 付与は protocol 全面改修になるため、往復計測は `request_id` を持つ request/reply ペアに限定する前提。
3. **GUI 未接続時の空白**（T032 / FR-003）: 現行 poller は `clients.has_clients()` が false の tick を丸ごと skip する。「常時収集」の定義をここで確定する必要がある。
