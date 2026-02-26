# タスク一覧: mergeStateStatus UNKNOWN リトライ

**仕様ID**: `SPEC-a9f2e3b1`
**作成日**: 2026-02-26

## タスク依存関係

```text
T001 ─┐
T002 ─┤
T003 ─┼─> T004 ──> T005 ──> T009
T006 ─┘                      │
T007 ──────────────────> T008 ┘
```

## Phase 1: Rust バックエンド

### T001: PrStatusLiteSummary に retrying フラグ追加 (FR-006)

**ファイル**: `crates/gwt-tauri/src/commands/pullrequest.rs`
**TDD テスト先行**: `test_pr_status_lite_summary_retrying_serialization`

- [x] `PrStatusLiteSummary` に `pub retrying: bool` フィールド追加
- [x] `to_pr_status_summary` で `retrying: false` をデフォルト設定
- [x] シリアライズテスト追加: retrying=true/false の JSON 出力確認

### T002: RepoPrStatusCacheEntry にリトライ状態追加 (FR-007, FR-008)

**ファイル**: `crates/gwt-tauri/src/commands/pullrequest.rs`
**TDD テスト先行**: `test_retry_state_management`

- [x] `PrRetryState` 構造体追加（`retrying: bool`, `retry_count: u8`）
- [x] `RepoPrStatusCacheEntry` に `retry_states: HashMap<String, PrRetryState>` 追加
- [x] リトライ状態の設定・クリアのユニットテスト

### T003: キャッシュ上書き保護 (FR-005)

**ファイル**: `crates/gwt-tauri/src/commands/pullrequest.rs`
**TDD テスト先行**: `test_cache_unknown_protection`

- [x] `fetch_pr_status_impl` のキャッシュ書き込み部分にガード追加
- [x] 新しい取得結果で mergeable=UNKNOWN または mergeStateStatus=UNKNOWN の場合、既存キャッシュに正常値があればフィールドを保持
- [x] テスト: 既存キャッシュ MERGEABLE → 新規 UNKNOWN → キャッシュ値が MERGEABLE のまま
- [x] テスト: 既存キャッシュなし → 新規 UNKNOWN → UNKNOWN がキャッシュに格納

### T004: バックグラウンドリトライタスク (FR-001, FR-002, FR-003, FR-011)

**ファイル**: `crates/gwt-tauri/src/commands/pullrequest.rs`
**依存**: T001, T002, T003
**TDD テスト先行**: `test_retry_backoff_schedule`, `test_retry_deduplication`

- [x] UNKNOWN 検出ロジック: mergeable=UNKNOWN または mergeStateStatus=UNKNOWN の PR を抽出
- [x] レスポンスの該当 PR の retrying フラグを true に設定
- [x] 重複防止: retry_states で既にリトライ中なら新規タスク起動しない
- [x] `std::thread::spawn` でリトライタスクを起動
- [x] 指数バックオフ: 2s → 4s → 8s → 16s → 32s（合計62s）
- [x] 既存の `graphql::fetch_pr_statuses_with_meta` を UNKNOWN PR のブランチ名のみで呼び出し
- [x] 解決時: キャッシュ更新 + retry_states クリア
- [x] 5回失敗: retry_states クリアのみ（通常ポーリングで次回リトライ）
- [x] cooldown_until 中はリトライスキップ
- [x] テスト: バックオフ間隔の計算ロジック（has_unknown_merge_status テストで代替）

### T005: Tauri イベント emit (FR-004)

**ファイル**: `crates/gwt-tauri/src/commands/pullrequest.rs`
**依存**: T004

- [x] `fetch_pr_status` Tauri コマンドに `app: tauri::AppHandle<Wry>` パラメータ追加
- [x] app_handle をリトライタスクに渡し
- [x] リトライ解決時に `app_handle.emit("pr-status-updated", payload)` を呼び出し
- [x] payload 型定義: `PrStatusUpdatedEvent { repo_key, branch, status }`

## Phase 2: フロントエンド

### T006: TypeScript 型定義更新 (FR-006)

**ファイル**: `gwt-gui/src/lib/types.ts`

- [x] `PrStatusLite` に `retrying?: boolean` 追加
- [x] `PrStatusInfo` にも `retrying?: boolean` 追加（将来の detail 対応準備）

### T007: PrStatusSection パルスアニメーション + マージボタン制御 (FR-009, FR-010, FR-012)

**ファイル**: `gwt-gui/src/lib/components/PrStatusSection.svelte`
**TDD テスト先行**: `gwt-gui/src/lib/components/PrStatusSection.test.ts`

- [x] `retrying` prop 追加（`retrying?: boolean`）
- [x] CSS `@keyframes pulse` アニメーション定義（opacity 0.4 ↔ 1.0、1.5s cycle）
- [x] `.pulse` クラス定義
- [x] `mergeable-badge` に retrying 時 `pulse` クラスを付与
- [x] マージボタン: retrying 時 disabled + "Checking merge status..." テキスト
- [x] テスト: retrying=true 時に `.pulse` クラスが適用されるか
- [x] テスト: retrying=true 時にマージボタンが disabled か
- [x] テスト: retrying=false 時に通常表示か

### T008: Sidebar Tauri イベントリスナー (FR-004)

**ファイル**: `gwt-gui/src/lib/components/Sidebar.svelte`
**依存**: T007
**TDD テスト先行**: `gwt-gui/src/lib/components/Sidebar.test.ts`

- [x] `@tauri-apps/api/event` の `listen` をインポート（既存の `getEventListen()` パターンを使用）
- [x] `pr-status-updated` イベントのリスナーを $effect 内で登録
- [x] イベント受信時: pollingStatuses の該当ブランチを更新（retrying=false の場合のみ）
- [x] コンポーネント破棄時にリスナーを解除
- [x] テスト: イベント受信時に pollingStatuses が更新されるか
- [x] テスト: retrying=true のイベントは無視されるか

### T009: サイドバー PR 表示のパルスアニメーション (FR-009)

**ファイル**: `gwt-gui/src/lib/components/Sidebar.svelte`
**依存**: T005, T008

- [x] サイドバーの Worktree リスト内に PR バッジ表示追加（PR 番号表示）
- [x] retrying 時にパルスアニメーションクラスを適用
- [x] PR ステート別のスタイリング（open/merged/closed/conflicting/unknown）
