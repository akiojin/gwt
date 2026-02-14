# 実装計画: Sidebar Filter Cache for Local/Remote/All

**仕様ID**: `SPEC-0f8e9c12` | **日付**: 2026-02-14 | **仕様書**: `specs/SPEC-0f8e9c12/spec.md`

## 目的

- Local/Remote/All 切替時の待ち時間を削減し、UIを即応化する
- キャッシュ表示と背景更新を両立して鮮度を維持する

## 方針

1. フィルター別キャッシュを `Sidebar.svelte` 内に保持
2. 切替時はキャッシュがあれば即時反映
3. 最終取得が10秒超過時のみ背景再取得を実行
4. `refreshKey` / `localRefreshKey` をキャッシュキーに含め、明示更新を優先
5. 同一キーの並列フェッチは in-flight map で重複排除

## 実装対象

- `gwt-gui/src/lib/components/Sidebar.svelte`
- `gwt-gui/src/lib/components/Sidebar.test.ts`

## 実装ステップ

### Step 1: データ取得層の分離

- `fetchFilterSnapshot(filter, path, cacheKey)` を追加し、取得結果をスナップショットとして返す
- `applyCacheEntry(entry)` を追加し、UI状態反映を単一経路化

### Step 2: フィルターキャッシュ制御

- `FilterCacheEntry` 型を追加
- `filterCache`（フィルター別）と `inflightFetches`（同一キー重複抑止）を追加
- TTL判定（10秒）を追加

### Step 3: 既存フェッチトリガー置換

- 既存 `fetchBranches()` を token 駆動のキャッシュ優先ロジックに置換
- キャッシュヒット時は `loading=false` を維持
- 背景フェッチ失敗時はキャッシュ表示を維持

### Step 4: テスト追加

- TTL内の再取得抑止テスト
- TTL超過時の背景再取得テスト
- 背景再取得中に `Loading...` を出さないテスト

## 検証

- `gwt-gui/src/lib/components/Sidebar.test.ts` を実行
- 既存の `refreshKey` テストが回帰していないことを確認
