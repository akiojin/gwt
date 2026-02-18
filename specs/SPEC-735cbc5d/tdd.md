# TDDノート: GitView Base 切り替え白画面不具合

**SPEC ID**: `SPEC-735cbc5d`  
**更新日**: 2026-02-18

## 対象

- `gwt-gui/src/lib/components/GitSection.svelte`
- `gwt-gui/src/lib/components/GitChangesTab.svelte`
- `gwt-gui/src/lib/components/GitCommitsTab.svelte`
- 各 `.test.ts`

## RED で固定する観点

1. Base 切り替え時に再取得が走ること
2. 旧リクエストの遅延応答（成功/失敗）が最新状態を上書きしないこと
3. 取得失敗が Git タブ内表示に閉じ、パネル全体を壊さないこと

## GREEN 方針

1. request-id ベースの latest-request-wins を 3 コンポーネントに導入
2. 基準ブランチ候補外値の防御を追加
3. stale 応答を明示的に破棄し、状態更新を最新要求のみに限定

## 実行ログ

- RED:
  - `GitSection.test.ts` に base 切り替え再取得 + stale failure 無視ケースを追加
  - `GitChangesTab.test.ts` / `GitCommitsTab.test.ts` に stale response 無視ケースを追加
- GREEN:
  - `GitSection.svelte` / `GitChangesTab.svelte` / `GitCommitsTab.svelte` へ request-id ガード導入
  - `GitSection.svelte` へ候補外 base 値フォールバックと再ロード時表示維持を追加
- VERIFY:
  - `pnpm -C gwt-gui check` => pass
  - `pnpm -C gwt-gui test src/lib/components/GitSection.test.ts src/lib/components/GitChangesTab.test.ts src/lib/components/GitCommitsTab.test.ts` => pass (20 tests)
