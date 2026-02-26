# 実装計画: mergeStateStatus UNKNOWN リトライ

**仕様ID**: `SPEC-a9f2e3b1`
**作成日**: 2026-02-26

## 概要

GitHub GraphQL API が返す一時的な UNKNOWN ステータスに対して、Rust バックエンドでバックグラウンドリトライを行い、フロントエンドにパルスアニメーション付きのローディング状態を表示する。

## 実装方針

### レイヤー構成

```text
[Frontend: Svelte]
  PrStatusSection.svelte  ── retrying フラグでパルスアニメーション
  Sidebar.svelte          ── Tauri イベントリスナー追加
  types.ts                ── retrying フィールド追加

[Backend: Rust (gwt-tauri)]
  pullrequest.rs          ── リトライロジック + キャッシュ保護 + イベント emit
```

### Phase 1: Rust バックエンド（キャッシュ保護 + リトライ機構）

#### 1-1. PrStatusLiteSummary / PrStatusResponse に retrying フラグ追加

- `PrStatusLiteSummary` に `retrying: bool` フィールド追加
- `to_pr_status_summary` で retrying 状態を反映

#### 1-2. RepoPrStatusCacheEntry にリトライ状態追加

- PR 単位のリトライ状態を管理する `PrRetryState` を追加
  - 保持項目: `retrying`（進行中フラグ）, `retry_count`（試行回数）, `branch_name`（対象ブランチ）
- `RepoPrStatusCacheEntry` に `retry_states: HashMap<String, PrRetryState>` 追加

#### 1-3. キャッシュ上書き保護

- `fetch_pr_status_impl` のキャッシュ書き込み部分で、取得した PR の mergeable/mergeStateStatus が UNKNOWN の場合:
  - 既存キャッシュに正常値があれば該当フィールドを上書きしない
  - 既存キャッシュが空（初回）なら UNKNOWN をそのまま格納

#### 1-4. バックグラウンドリトライタスク

- `fetch_pr_status_impl` が UNKNOWN PR を検出したとき:
  1. レスポンスの該当 PR に `retrying=true` を設定
  2. 既にリトライ中でなければ `std::thread::spawn` でリトライタスクを起動
  3. リトライタスク内で指数バックオフ（2s, 4s, 8s, 16s, 32s）でループ
  4. 既存の `build_pr_status_query` を UNKNOWN PR のブランチ名のみで呼び出し
  5. UNKNOWN が解決したらキャッシュ更新 + リトライ状態クリア
  6. 5回到達でリトライ状態をクリアし終了

#### 1-5. Tauri イベント emit

- リトライ成功時に `app_handle.emit("pr-status-updated", payload)` でフロントエンドに通知
- payload: `{ repoKey: String, branch: String, status: PrStatusLiteSummary }`
- `app_handle` へのアクセス: `fetch_pr_status` の Tauri コマンド関数から `tauri::AppHandle` を受け取り、リトライタスクに渡す

### Phase 2: フロントエンド（UI 更新）

#### 2-1. TypeScript 型定義更新

- `PrStatusLite` に `retrying?: boolean` 追加
- `PrStatusResponse` の statuses 内の各 PR にも反映

#### 2-2. Tauri イベントリスナー

- Sidebar.svelte のポーリングロジック内で `listen("pr-status-updated")` を追加
- イベント受信時に該当ブランチの `pollingStatuses` を即座更新

#### 2-3. パルスアニメーション

- CSS アニメーション `@keyframes pulse` を定義
- PrStatusSection.svelte:
  - `retrying` prop を追加
  - `mergeable-badge` に `retrying` 時 `pulse` クラスを付与
  - マージボタン: `retrying` 時は disabled + "Checking merge status..." 表示
- Sidebar.svelte:
  - サイドバーのPR表示要素に `retrying` 時 `pulse` クラスを付与

### Phase 3: テスト

#### 3-1. Rust ユニットテスト

- キャッシュ保護: UNKNOWN でキャッシュが上書きされないことを検証
- リトライ状態管理: retrying フラグの設定・クリアを検証
- `PrStatusLiteSummary` の retrying フィールドのシリアライズ検証

#### 3-2. フロントエンド テスト

- PrStatusSection: retrying 時のパルスアニメーションクラス適用
- PrStatusSection: retrying 時のマージボタン disabled + テキスト変更
- Sidebar: イベントリスナーによるステータス更新

## 変更対象ファイル

| ファイル | 変更内容 |
|---------|---------|
| `crates/gwt-tauri/src/commands/pullrequest.rs` | リトライロジック、キャッシュ保護、retrying フラグ、イベント emit |
| `gwt-gui/src/lib/types.ts` | PrStatusLite に retrying 追加 |
| `gwt-gui/src/lib/components/PrStatusSection.svelte` | パルスアニメーション、マージボタン制御 |
| `gwt-gui/src/lib/components/Sidebar.svelte` | Tauri イベントリスナー追加 |
| `crates/gwt-tauri/src/commands/pullrequest.rs` (tests) | ユニットテスト追加 |
| `gwt-gui/src/lib/components/PrStatusSection.test.ts` | フロントエンドテスト追加 |
| `gwt-gui/src/lib/components/Sidebar.test.ts` | イベントリスナーテスト追加 |

## リスクと軽減策

| リスク | 軽減策 |
|-------|-------|
| リトライによる API レート制限圧迫 | cooldown_until チェック + 最大5回制限 |
| リトライタスクのリーク | retry_count 上限 + retrying フラグによる重複防止 |
| app_handle のライフタイム管理 | Tauri の managed state 経由でクローンして渡す |
