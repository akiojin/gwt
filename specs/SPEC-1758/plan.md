<!-- GWT_SPEC_ARTIFACT:doc:plan.md -->
doc:plan.md

## Summary

profiling (`#1705`) と通常ログ基盤を責務分離したまま、通常ログを「機能ログ」と「障害対応ログ」の 2 軸で再設計する。既存の `tracing` 呼び出しを横断的に整理し、主要機能フローと主要障害シナリオの 90%以上でログだけから追跡・一次切り分けできる状態を目標にする。

## Technical Context

### 現状アーキテクチャ

- `crates/gwt-core/src/logging/logger.rs` が `gwt.jsonl` の日次ローテーションと profiling 用 `profile.json` を同居させて初期化している
- `crates/gwt-tauri/src/main.rs` が global settings を読み、起動時に logging を初期化する
- `crates/gwt-tauri/src/commands/report.rs` が `read_recent_logs` で jsonl 系のみを探索し、Report Dialog に渡している
- `gwt-gui/src/lib/diagnostics.ts` と `gwt-gui/src/lib/components/ReportDialog.svelte` が `Application Logs` の収集導線を持つ
- `tracing::{debug,info,warn,error}` 呼び出しは `project`, `terminal`, `assistant`, `settings`, `issue`, `migration`, `docker`, `worktree`, `app` などに散在しており、命名規約とカバレッジ基準が統一されていない

### 影響ファイル / モジュール

- `crates/gwt-core/src/logging/logger.rs`, `crates/gwt-core/src/logging/mod.rs`, `crates/gwt-core/src/logging/reader.rs`
- `crates/gwt-tauri/src/main.rs`, `crates/gwt-tauri/src/app.rs`, `crates/gwt-tauri/src/commands/report.rs`
- 高優先 subsystem: `crates/gwt-tauri/src/commands/project.rs`, `terminal.rs`, `assistant.rs`, `settings.rs`, `issue.rs`, `system.rs`
- 高優先 core modules: `crates/gwt-core/src/worktree/manager.rs`, `git/repository.rs`, `git/branch.rs`, `docker/manager.rs`, `config/settings.rs`, `migration/executor.rs`
- GUI integration: `gwt-gui/src/lib/diagnostics.ts`, `gwt-gui/src/lib/components/ReportDialog.svelte`

### 前提と制約

- profiling は別 SPEC (`#1705`) の責務として維持し、この SPEC では通常ログ jsonl のみ扱う
- coverage 90% はコード行数ではなく、機能/障害シナリオの matrix で判定する
- GUI の全操作イベントは対象外。backend 呼び出し境界と障害報告導線のみ対象とする
- 既存 log path / report path / privacy mask の互換性を壊さない

## Constitution Check

| Rule | Status |
|------|--------|
| Spec Before Implementation | ✅ 仕様・計画・タスク・分析を整えてから実装へ進む |
| Test-First Delivery | ✅ 機能ログ/障害ログ coverage はテストと matrix で先に固定する |
| No Workaround-First Changes | ✅ 既存ログの散在・不統一を taxonomy と coverage 基準で直接整理する |
| Minimal Complexity | ✅ 新規ログ基盤を増やすのではなく、既存 `tracing` と report 経路を整理する |
| Verifiable Completion | ✅ acceptance scenario と coverage matrix で完了判定を定量化する |

### Required Plan Gates

1. **影響ファイル/モジュール**: logging 初期化、report 収集、主要 subsystem の `tracing` 呼び出し群
2. **適用される constitution ルール**: test-first、spec-first、minimal complexity
3. **受容する複雑性**: subsystem 横断 taxonomy と coverage matrix の追加
4. **検証方法**: unit/integration test + report 導線検証 + matrix レビュー

## Project Structure

```text
crates/gwt-core/src/logging/
├── logger.rs
├── mod.rs
└── reader.rs

crates/gwt-tauri/src/
├── main.rs
├── app.rs
└── commands/
    ├── report.rs
    ├── project.rs
    ├── terminal.rs
    ├── assistant.rs
    ├── settings.rs
    ├── issue.rs
    └── system.rs

gwt-gui/src/lib/
├── diagnostics.ts
└── components/ReportDialog.svelte
```

## Complexity Tracking

| 追加事項 | 理由 | 代替案と棄却理由 |
|---------|------|------------------|
| coverage matrix の導入 | 90% をコード行数でなく業務的に判定するため | 単純な grep/count は acceptance を保証できないため棄却 |
| subsystem 横断 taxonomy | `category/event/result/error` の意味を揃えるため | 各 module 任せでは障害解析の一貫性が出ないため棄却 |
| GUI の細粒度イベントを対象外にする | 初版でノイズと工数を抑えるため | 全 GUI telemetry を含めると 90% 定義が不安定になるため棄却 |

## Phased Implementation

### Phase 1: Baseline audit and taxonomy

- 既存 logging/report/profiling の責務分離をコード上で棚卸しする
- 主要 subsystem ごとの `category/event/result/error` taxonomy を決める
- 機能ログ matrix と障害対応ログ matrix の対象一覧を作成する

### Phase 2: Logging contract and shared helpers

- 通常ログの必須/条件付きフィールド契約を定義する
- 高頻度 subsystem に適用しやすい共通 helper / pattern を決める
- `read_recent_logs` と report 導線が通常ログのみ扱うことを明文化する

### Phase 3: Feature-flow coverage

- app lifecycle, project, settings, issue/report, agent, worktree, git/pr, docker, migration を対象に `start/success/failure` の不足箇所を補う
- feature coverage matrix を 90%以上まで引き上げる
- 既存 message と structured field の整合を確認する

### Phase 4: Incident-response coverage

- 主要 failure path に `context/error_detail/error_code` を追加する
- 外部依存失敗（git/github/docker/network/process/fs）と内部失敗を切り分けられるようにする
- incident coverage matrix を 90%以上まで引き上げる

### Phase 5: Report / validation / docs

- Issue report で通常ログのみが収集されることを保証する
- coverage matrix と acceptance scenario に基づく test / review 手順を整える
- profiling との責務分離を README または reviewer 向け quickstart に明記する
