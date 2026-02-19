# 実装計画: Sidebar Branch Switch Non-Blocking

**仕様ID**: `SPEC-c0b4e2d9` | **日付**: 2026-02-18 | **仕様書**: `specs/SPEC-c0b4e2d9/spec.md`

## 目的

- サイドバーのブランチ切替操作で体感フリーズをなくす。
- 重いデータ取得をタブ表示時に限定して、無駄なバックエンド呼び出しを削減する。

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-tauri/`）
- **フロントエンド**: Svelte 5 + TypeScript（`gwt-gui/`）
- **ストレージ/外部連携**: gh CLI（PR参照）
- **テスト**: `pnpm test`（vitest）, `cargo test`
- **前提**: 既存6タブUIとSummaryポーリング仕様を維持する

## 実装方針

### Phase 1: WorktreeSummaryPanel の取得責務分離

- `selectedBranch` 変更時の一括取得を廃止し、Summary関連のみ即時取得。
- Issue/PR/Docker は active tab 条件で遅延取得。
- branch key/token ガードを維持し、stale混入を防止。

### Phase 2: フロント側キャッシュ + 1フレーム遅延

- branch単位メモリキャッシュを導入（Issue/PR/Docker + Quick Start）。
- TTLを設定し、同一ブランチ往復時の再取得を抑制。
- 重い取得は `requestAnimationFrame` 後に開始し、選択描画を優先。

### Phase 3: backend PR参照抑制

- `fetch_latest_branch_pr` に短TTLキャッシュを追加。
- `gh pr list` 連続実行を防止し、切替連打時の負荷を低減。

## テスト

### バックエンド

- `fetch_latest_branch_pr` の既存機能回帰がないことを確認（`cargo test`）。

### フロントエンド

- `WorktreeSummaryPanel.test.ts` に以下を追加/更新:
  - ブランチ切替直後に Issue/PR/Docker API が走らない
  - タブ初回表示時のみ遅延取得される
  - 同一ブランチ再選択時にTTL内キャッシュ再利用
  - stale応答が表示汚染しない
