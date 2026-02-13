# 調査: Ink.js から OpenTUI への移行

## 既存コードベースの把握

- UI の主な配置は `src/cli/ui/` 配下。
- 画面（Screen）実装は `src/cli/ui/components/screens/` と `src/cli/ui/screens/` に存在。
- 画面切り替えや UI 全体制御は `src/cli/ui/components/App.tsx` に集約。
- 共通 UI 部品は `src/cli/ui/components/common/` と `src/cli/ui/components/parts/` に集約。
- 入力やナビゲーションは `src/cli/ui/hooks/` のカスタムフックに依存。

## 既存テスト/性能テスト

- UI テストは `src/cli/ui/__tests__/` 配下に大量に存在（主に Vitest）。
- Ink.js 固有のテスト補助として `ink-testing-library` を利用。
- パフォーマンステストは `src/cli/ui/__tests__/performance/branchList.performance.test.tsx` に存在。
- Web 向けの E2E は `tests/e2e`（Playwright）に存在。

## Ink.js 性能ベースライン（測定結果）

測定日: 2026-01-04  
測定コマンド: `bun run test -- src/cli/ui/__tests__/performance/branchList.performance.test.tsx`  
測定環境: ローカル開発環境（CI ではない）

- 150 branches render: 44.10ms（平均 0.294ms/branch）
- Re-render（stats 更新）: 31.84ms
- 250 branches render: 11.90ms

※ 測定値はローカル環境依存のため、OpenTUI 移行後も同一条件で再測定し比較する。

### 追加測定（5000ブランチ）

測定日: 2026-01-05  
測定コマンド: `bun run test -- src/cli/ui/__tests__/performance/branchList.performance.test.tsx`  
測定環境: ローカル開発環境（CI ではない）

- 150 branches render: 41.35ms（平均 0.276ms/branch）
- Re-render（stats 更新）: 27.42ms
- 250 branches render: 11.46ms
- 5000 branches render: 11.48ms
- 入力レイテンシ（Downキー x5 の平均）: 88.97ms（簡易測定）
- 参考: 入力更新の概算 FPS: 11.2

※ 入力レイテンシは ink-testing-library のフレーム更新待ちで測定した簡易値。実端末のスクロール FPS とは一致しない可能性がある。

## OpenTUI BranchListScreen 性能測定（測定結果）

測定日: 2026-01-05  
測定コマンド: `bun test --preload @opentui/solid/preload src/cli/ui/__tests__/solid/BranchListScreen.performance.test.tsx`  
測定環境: ローカル開発環境（CI ではない）

- 5000 branches render: 8.61ms
- 入力レイテンシ（Downキー x5 の平均）: 1.06ms
- 参考: 入力更新の概算 FPS: 945.3

※ OpenTUI のテスト実行には `@opentui/solid` の Bun プラグインが必要なため、`--preload` を指定している。

## BranchListScreen 機能パリティ確認（Ink.js vs OpenTUI）

確認日: 2026-01-05  
対象: `src/cli/ui/components/screens/BranchListScreen.tsx`（Ink.js）と `src/cli/ui/screens/solid/BranchListScreen.tsx`（OpenTUI）

### 結果

OpenTUI 版は **主要機能と細部表示のパリティを達成**。

- 色付け/強調表示（ツールラベル/警告色/cleanup インジケータ）: 対応済み
- cleanup フッターメッセージのスピナーアニメーション: 対応済み
- フィルター入力のカーソル表示: 対応済み
- DEBUG 時のエラースタック詳細表示: 対応済み

### Go/No-Go 判定（US1）

- 判定: Go
- 根拠: 5000ブランチ描画/入力レイテンシの測定結果が目標（60fps/16ms）を満たす

### Go/No-Go 判定（US3 / 中間）

- 判定日: 2026-01-05
- 判定: Go
- 根拠: US3（Loading/Confirm/Input/Error）の移行と統合テスト完了、主要スクリーン群の移行が進捗しブロッカーなし

## OpenTUI 最終性能ベンチマーク（5000ブランチ）

測定日: 2026-01-05  
測定コマンド: `bun test --preload @opentui/solid/preload src/cli/ui/__tests__/solid/BranchListScreen.performance.test.tsx`  
測定環境: ローカル開発環境（CI ではない）

- 5000 branches render: 15.83ms
- 入力レイテンシ（Downキー x5 の平均）: 4.73ms
- 参考: 入力更新の概算 FPS: 211.6

※ 選択行の視覚差分が色のみになったため、フレーム差分待ちではなく `renderOnce()` による描画時間を入力レイテンシとして計測。

## 技術的決定

1. OpenTUI + SolidJS を採用し、Ink.js 依存を最終リリースで撤去する。
2. Zig コンパイラは開発/CI で必須とし、配布物にはネイティブバイナリを同梱する。
3. 既存の UI テストと性能ベンチを移行・維持し、ベースラインから悪化しないことを品質ゲートにする。
4. Windows ネイティブ対応を前提とし、動作確認を必須ゲートに含める。

## 移行の影響範囲

- 画面単位の移行が主軸。`App.tsx` の画面ルーティングと状態管理は影響が大きい。
- 共通 UI 部品は OpenTUI のコンポーネントに合わせて再構成が必要。
- Ink.js 固有の入出力/描画に依存するテストは OpenTUI 向けへ置換が必要。

## 制約/依存関係

- パフォーマンス/入力遅延のベースライン維持が必須。
- Windows ネイティブでの動作保証が必須。
- CLI 出力とログは分離（specs/SPEC-b9f5c4a1 参照）。
- 仕様上、最終リリースで Ink.js 依存は残さない。
