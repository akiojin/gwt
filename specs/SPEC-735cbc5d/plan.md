# 実装計画: GitView Base 切り替え白画面不具合修正

**SPEC ID**: `SPEC-735cbc5d`  
**更新日**: 2026-02-18  
**対象**: `gwt-gui/src/lib/components/GitSection.svelte` / `GitChangesTab.svelte` / `GitCommitsTab.svelte`

## 目的

- `Base` を `main` から `develop` に切り替えた際、Worktree Summary 全体が消失する不具合を解消する。
- 非同期取得競合（stale response）で古い結果が新しい選択を上書きしない状態遷移へ統一する。
- 取得失敗時の影響範囲を Git タブ内に閉じ込める。

## 方針

1. **latest-request-wins** を `GitSection` / `GitChangesTab` / `GitCommitsTab` に適用する。
2. 基準ブランチ変更イベントは候補値のみ受け入れ、候補外入力はフォールバックする。
3. 失敗時はコンポーネント内エラー表示に留め、例外を上位へ伝播させない。

## 実装ステップ

### Phase 1: TDD（RED）

- `GitSection.test.ts`
  - base 変更時に summary が再取得されること
  - 旧リクエスト失敗が最新状態を上書きしないこと
- `GitChangesTab.test.ts`
  - base 変更時に古い diff 一覧応答が無視されること
- `GitCommitsTab.test.ts`
  - base 変更時に古い commit 一覧応答が無視されること

### Phase 2: 実装（GREEN）

- `GitSection.svelte`
  - summary 取得に request-id ガードを導入
  - base 候補と現在値の整合チェック + フォールバック
  - `handleBaseBranchChange` を安全化（候補外値拒否）
- `GitChangesTab.svelte`
  - `loadFiles` に request-id ガードを導入
  - stale 結果を破棄して状態競合を防止
- `GitCommitsTab.svelte`
  - `load` / `loadMore` に request-id ガードを導入
  - stale 結果を破棄して状態競合を防止

### Phase 3: 検証

- `pnpm -C gwt-gui test src/lib/components/GitSection.test.ts src/lib/components/GitChangesTab.test.ts src/lib/components/GitCommitsTab.test.ts`
- `pnpm -C gwt-gui check`

## 影響範囲

- 公開 API/IPC コマンドの変更はなし。
- フロントエンド内部状態遷移のみ変更。

## 受け入れ条件

- `main -> develop` 切り替え時に白画面化しない。
- 取得失敗は Git タブ内に限定表示される。
- 連続切り替え時に最終選択結果のみ表示される。
