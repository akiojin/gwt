# クイックスタート: semantic-release リリースワークフローの安定化

**仕様ID**: `SPEC-78e66d9a`  
**作成日**: 2025-11-05

## 1. 事前準備

- Bun 1.0 以上がインストール済みであること。
- 依存関係をインストール:

```bash
bun install
```

## 2. 再現手順

1. `main` ブランチ相当のコードで以下を実行:

```bash
bun run test src/ui/__tests__/components/common/LoadingIndicator.test.tsx
```

2. CI で失敗したケースではタイマー遅延によりスピナーのフレームが更新されずテストが失敗する。ローカルで再現しづらい場合は `--repeat 20` を付与して繰り返し実行する。

## 3. 修正後の検証

### ローカル

```bash
bun run test src/ui/__tests__/components/common/LoadingIndicator.test.tsx
```

- 疑似タイマーを利用したテストが安定して通過することを確認。
- 追加で `bun run test` を実行して全体テストを確認。

### GitHub Actions

1. Pull Request を作成して `release` ワークフローをトリガー。
2. ワークフローページで `Test` ジョブが成功し、`semantic-release` まで到達することを確認。
3. 必要に応じてワークフローを手動再実行しても成功することを確認。

## 4. トラブルシューティング

- 疑似タイマーが正しく機能しない場合は `vi.useFakeTimers({ shouldAdvanceTime: true })` の利用を検討。
- Ink のレンダリングが更新されない場合は `await act(() => vi.advanceTimersByTimeAsync(x))` を用いて DOM 更新を待機。
- テストがハングする場合は `vi.useRealTimers()` が適切に呼ばれているか確認。
