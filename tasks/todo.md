# TODO: Issue #1256 Modal Front Layering

## 背景
Issue #1256「エラーリポートウィンドウを一番手前に表示」を、既存 `SPEC-fabb6678` に追記し、共通モーダル層設計と TDD（Unit + E2E）で修正する。

## 実装ステップ
- [x] SPEC-fabb6678（spec/plan/tasks/tdd）へ US10 + FR + 検証計画を追記
- [x] モーダル共通 z-index トークンを導入し、モーダル関連コンポーネントへ適用
- [x] ReportDialog 最前面保証の Unit テストを追加
- [x] モーダル競合時の最前面保証 E2E を追加
- [x] テスト/チェック実行（ReportDialog unit, dialogs-common e2e, svelte-check）

## 検証結果
- [x] `cd gwt-gui && pnpm test src/lib/components/ReportDialog.test.ts`（pass: 27 tests）
- [x] `cd gwt-gui && pnpm exec playwright test e2e/dialogs-common.spec.ts --project=chromium`（pass: 11 tests）
- [x] `cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json`（0 errors / 1 warning）
