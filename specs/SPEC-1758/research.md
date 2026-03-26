<!-- GWT_SPEC_ARTIFACT:doc:research.md -->
doc:research.md

## Existing State Findings

- `logger.rs` は通常ログ (`gwt.jsonl`) と profiling (`profile.json`) を同じ workspace log dir に生成するが、出力契約は profiling 優先で整理されていない
- `report.rs` の `read_recent_logs` は `.jsonl` 系のみを探索するため、現状でも profiling は Issue report に含めていない
- `diagnostics.ts` / `ReportDialog.svelte` は通常ログ収集の UI 導線を持つ
- `tracing` 呼び出しは多くの subsystem に存在するが、どの機能を coverage 対象とするかは未定義

## Spec Integration Search

- `gwt-issue-search` を 2 クエリで実行したが、logging foundation を canonical に扱う既存 `gwt-spec` は見つからなかった
- 近い候補は `#1705` (profiling) のみであり、通常ログとは責務が異なるため新規 SPEC とする
- 既存の関連 issue として `#1448` (`Application Logsが機能しない`) があるが、これは report 導線の不具合修正であり logging foundation 全体は未仕様化

## Decision Record

1. **profiling と通常ログは分離を維持する**
   - 理由: ツール、形式、利用者、診断目的が異なるため
2. **90% は line coverage ではなく scenario coverage で定義する**
   - 理由: 運用価値と acceptance に直結するため
3. **初版対象は Core + Tauri 中心、GUI は report 導線のみ**
   - 理由: 粒度を揃えやすく、最小複雑性で進められるため
4. **機能ログと障害対応ログを別 matrix で管理する**
   - 理由: 成功系トレースと failure triage では必要情報が異なるため

## Open Risks

- subsystem 横断で message 文面が揃っておらず、taxonomy 導入時に既存ログとの差分が大きくなる可能性がある
- `terminal.rs` や `migration/executor.rs` のような高密度 logging 領域では、ノイズ抑制と障害情報の両立が難しい
- privacy / masking の境界を明文化しないと、障害対応ログ強化で機微情報混入の危険がある

## Recommended Direction

- まず coverage matrix を先に固定し、高優先 subsystem に限定して gap analysis を行う
- structured field 契約を `category/event/result/error` で最小限統一し、message 文面の全面統一は後回しにする
- Issue report との接続性を acceptance に含め、通常ログ基盤のゴールを運用起点で評価する
