# タスクリスト: Sidebar Filter Cache for Local/Remote/All

## Phase 1: 仕様反映

- [x] T001 `spec.md` 作成（受け入れシナリオ・FR/NFR・成功基準）
- [x] T002 `plan.md` 作成（実装方針・ステップ分解）

## Phase 2: TDD（RED → GREEN → REFACTOR）

- [x] T010 [RED] `Sidebar.test.ts` に TTL内キャッシュ再利用テストを追加（失敗確認）
- [x] T011 [RED] `Sidebar.test.ts` に TTL超過時の背景再取得テストを追加（失敗確認）
- [x] T012 [GREEN] `Sidebar.svelte` にフィルター別キャッシュ + TTL + in-flight 重複抑止を実装
- [x] T013 [GREEN] テストを通し、既存の `refreshKey` テスト回帰なしを確認
- [x] T014 [REFACTOR] 取得・適用・エラー変換ロジックを関数分割して可読性を改善
- [x] T015 [RED] `Sidebar.test.ts` に「フィルター切替で `fetch_pr_status` が即時再実行されない」テストを追加（失敗確認）
- [x] T016 [GREEN] `Sidebar.svelte` の PR ポーリングを filter 切替非依存 + in-flight 抑止に修正
- [x] T017 [GREEN] `pnpm -s vitest run src/lib/components/Sidebar.test.ts` を通過させる

## Phase 3: 仕上げ

- [x] T020 手動観点（切替即応・背景更新・エラー時維持）を確認
- [x] T021 変更ファイルをレビューし、不要差分がないことを確認
