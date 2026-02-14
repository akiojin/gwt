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
- [x] T018 [RED] `Sidebar.test.ts` に「検索入力フォーカス中は30秒周期 `fetch_pr_status` をスキップ」テストを追加（失敗確認）
- [x] T019 [RED] `Sidebar.test.ts` に「検索入力がデバウンス後に反映される」テストを追加（失敗確認）
- [x] T020 [GREEN] `Sidebar.svelte` に入力中ポーリング抑止 + 検索デバウンスを実装
- [x] T021 [GREEN] `pnpm -s vitest run src/lib/components/Sidebar.test.ts` を通過させる
- [x] T022 [RED] `Sidebar.test.ts` に「projectPath切替時にin-flightが別プロジェクトを阻害しない」テストを追加（失敗確認）
- [x] T023 [RED] `Sidebar.test.ts` に「projectPath切替時に旧PRバッジが即時クリアされる」テストを追加（失敗確認）
- [x] T024 [RED] `Sidebar.test.ts` に「ブランチロード後にPRポーリングが即時ブートストラップされる」テストを追加（失敗確認）
- [x] T025 [GREEN] `Sidebar.svelte` に projectPath 単位 in-flight + stale state reset + bootstrap改善を実装

## Phase 3: 仕上げ

- [x] T030 手動観点（切替即応・背景更新・エラー時維持）を確認
- [x] T031 変更ファイルをレビューし、不要差分がないことを確認
