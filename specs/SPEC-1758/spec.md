<!-- GWT_SPEC_ARTIFACT:doc:spec.md -->
doc:spec.md

# Feature Specification: 通常ログ基盤の統合設計と機能・障害対応ログ90%カバレッジ

## Background

現在の gwt には JSON Lines 形式の通常ログ、Issue report の `Application Logs` 収集、`profile.json` を用いる profiling がそれぞれ存在するが、通常ログの責務・粒度・カバレッジ基準は仕様として統一されていない。`#1705` は profiling 用 SPEC であり、障害解析や運用追跡のための通常ログは別スコープとして定義する必要がある。

本仕様では、profiling を Chrome Trace 用の別出力と維持したまま、通常ログ基盤を「機能ログ」と「障害対応ログ」の観点で再定義する。主要求は、主要機能フローと主要障害シナリオの 90%以上で、ログだけから実行結果と一次切り分けができる状態にすることである。

## Scope Definition

- **対象**: `gwt-core` / `gwt-tauri` の logging 境界、Tauri command 境界、Issue report 収集経路、主要バックエンド機能フロー
- **部分対象**: GUI は backend 呼び出し境界と Issue report 導線のみ
- **非対象**: Chrome Trace profiling、細粒度 UI telemetry、全クリック/hover のイベント収集

## Terminology

- **通常ログ**: `gwt.jsonl` 系列の JSON Lines ログ。運用・障害解析・Issue report 用
- **profiling**: `profile.json` に出力される Chrome Trace。性能解析専用
- **機能ログ coverage**: 主要機能フローのうち、開始・成功・失敗がログから追跡できる割合
- **障害対応ログ coverage**: 主要障害シナリオのうち、原因特定に必要な context と failure detail が残る割合
- **coverage matrix**: 機能/障害シナリオごとに必要ログ点を定義した判定表

## User Stories

### US-1: 主要機能フローをログだけで追跡したい (Priority: P0)
- Given: ユーザーが主要機能を実行する
- When: 後から通常ログを確認する
- Then: 開始、主要状態遷移、成功/失敗を時系列で追跡できる

### US-2: 障害発生時に一次切り分けしたい (Priority: P0)
- Given: Git, Docker, Agent, filesystem, network, command 実行のいずれかで失敗が起きる
- When: 運用者または開発者が通常ログを確認する
- Then: failure point、原因カテゴリ、関連コンテキスト、復旧判断に必要な情報を取得できる

### US-3: Issue report に通常ログだけを安全に含めたい (Priority: P0)
- Given: ユーザーが Report Dialog から不具合報告を作成する
- When: `Application Logs` を添付する
- Then: profiling 出力を混ぜず、通常ログのみを必要行数で収集できる

### US-4: ログ粒度と責務を subsystem 横断で揃えたい (Priority: P1)
- Given: 複数 subsystem が `tracing` を使っている
- When: ログを横断検索する
- Then: `category` / `event` / `result` / `error` の意味が統一され、比較可能である

## Acceptance Scenarios

1. Given アプリ起動、When `open_project` まで処理が進む、Then 通常ログだけで startup から project open の成功/失敗を追跡できる
2. Given settings 保存、When `save_settings` が成功または失敗する、Then 設定更新の結果と失敗理由が通常ログに残る
3. Given Agent / terminal / external process 起動失敗、When 通常ログを確認する、Then subsystem、コマンド、失敗結果、原因を一次切り分けできる
4. Given git / GitHub / Docker 操作失敗、When 通常ログを確認する、Then branch / issue / repository context を含めて失敗点を特定できる
5. Given Report Dialog で `Application Logs` を収集する、When report body を生成する、Then `gwt.jsonl` 系の通常ログのみが対象となり `profile.json` は含まれない
6. Given profiling が有効、When 通常ログを読む、Then profiling の有無に関係なく通常ログの構造と収集経路は維持される
7. Given coverage matrix に定義された主要機能フロー、When 仕様レビューを行う、Then 90%以上のフローで `start/success/failure` のログ点が定義されている
8. Given coverage matrix に定義された主要障害シナリオ、When 仕様レビューを行う, Then 90%以上のシナリオで `category/event/context/error` が定義されている

## Edge Cases

- profiling 有効時でも通常ログ収集 API は `profile.json` を対象にしてはならない
- 機密情報や長大な payload は通常ログへそのまま出力してはならない
- 失敗時の context 付与は強化するが、成功時ログまで verbose にし過ぎてノイズを増やしてはならない
- GUI の細粒度イベントは coverage 対象から除外し、backend 呼び出し境界のみ扱う
- 既存 subsystem ごとの自由な message 文面は許容するが、最低限の構造化フィールドは統一しなければならない
- `read_recent_logs` は workspace 配下の最新 jsonl を返す現在仕様を維持しつつ、対象種別の混在を防ぐ必要がある

## Functional Requirements

- FR-001: 通常ログ基盤は profiling と別責務として定義されなければならない
- FR-002: 主要機能フローの 90%以上で `start` / `success` / `failure` のログ点を定義しなければならない
- FR-003: 主要障害シナリオの 90%以上で `category` / `event` / `context` / `error` を含む障害対応ログを定義しなければならない
- FR-004: 通常ログの必須フィールドは少なくとも `category`, `event`, `result`, `workspace` を含まなければならない
- FR-005: 条件付きフィールドとして `project_path`, `branch`, `issue_id`, `session_id`, `command`, `duration_ms`, `error_code`, `error_detail` を subsystem ごとに使い分けなければならない
- FR-006: Report Dialog の `Application Logs` は通常ログのみを収集し、profiling 出力を含めてはならない
- FR-007: coverage 判定はコード行数ではなく、SPEC に定義した coverage matrix に基づかなければならない
- FR-008: coverage matrix は少なくとも app lifecycle, project, settings, issue/report, agent, worktree, git/pr, docker, migration の各領域を含まなければならない
- FR-009: subsystem ごとにログ taxonomy を定義し、`category` と `event` の意味が横断的に解釈できなければならない
- FR-010: 既存 `tracing` 呼び出しが多い高価値領域から優先的に整備し、GUI の細粒度 telemetry は対象外としなければならない

## Non-Functional Requirements

- NFR-001: 通常ログは JSON Lines 形式と既存保存場所を維持しなければならない
- NFR-002: profiling 有効時でも通常ログの収集・Issue report 導線に影響を与えてはならない
- NFR-003: ログ追加により通常経路の可読性を損なう過剰ノイズを生じさせてはならない
- NFR-004: 障害対応ログは、一次切り分けに必要な情報を 1 回の log collection で取得できる粒度でなければならない
- NFR-005: 機密値や個人情報は既存 masking 方針と矛盾しない形で扱わなければならない
- NFR-006: coverage matrix はレビュー時に定量判定できる形で維持されなければならない

## Success Criteria

- SC-001: 主要機能フロー coverage が 90%以上であることを coverage matrix で示せる
- SC-002: 主要障害シナリオ coverage が 90%以上であることを coverage matrix で示せる
- SC-003: profiling と通常ログの責務分離が仕様・実装・Issue report 導線で一貫している
- SC-004: 主要 subsystem で構造化フィールドが統一され、横断的な検索/読解ができる
- SC-005: Issue report の `Application Logs` から、主要障害シナリオの一次切り分けに必要な情報を採取できる
