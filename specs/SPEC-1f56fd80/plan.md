# Plan: Web UI システムトレイ統合とURL表示

**仕様ID**: `SPEC-1f56fd80`

## 実装方針

1. Web UI サーバー起動完了フックでトレイ初期化を行う（失敗は握りつぶして警告のみ）
2. 軽量なクロスプラットフォームのトレイライブラリを採用し、アイコンとダブルクリックアクションを設定
3. 既定ブラウザで URL を開くための小さなユーティリティを追加
4. BranchListScreen に Web UI URL 行を追加し、レイアウト行数を調整
5. 仕様に基づくユニットテストを先に追加し、グリーン化する

## 影響範囲

- サーバー: `src/web/server/index.ts` + 新規トレイモジュール
- CLI UI: `src/cli/ui/components/screens/BranchListScreen.tsx`
- 依存: 新規 npm 依存（軽量トレイライブラリ）
- テスト: `tests/unit` と `src/cli/ui/__tests__`
