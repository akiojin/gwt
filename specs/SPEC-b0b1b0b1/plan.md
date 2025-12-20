# 実装計画: AIツール終了時の未コミット警告と未プッシュ確認

**仕様ID**: `SPEC-b0b1b0b1` | **日付**: 2025-12-20 | **仕様書**: [spec.md](./spec.md)

## 方針
- `handleAIToolWorkflow` の終了直前にGit状態チェックと確認プロンプトを挿入する。
- プロンプトは `getTerminalStreams` を使用し、非TTYは即時デフォルト（No）で返す。
- TDDでプロンプトと終了時分岐の挙動を先に固める。

## ステップ
1. **TDD**: `confirmYesNo` のTTY/非TTY挙動テストを追加（`src/utils/__tests__/prompt.test.ts`）。
2. **実装**: `confirmYesNo` を `src/utils/prompt.ts` に追加。
3. **TDD**: 終了時の未コミット警告・未プッシュ確認・push分岐のテストを追加（`tests/unit/index.post-session-checks.test.ts`）。
4. **実装**: `src/index.ts` に終了時チェックを追加し、3秒待機の順序を調整。
5. **補正**: 既存の `handleAIToolWorkflow` テストで新プロンプトをモック。
6. **検証**: `bun run test`（必要に応じて該当テストのみに絞る）。

## リスクと緩和
- **リスク**: TTY入力待ちがテストをブロックする  
  **緩和**: 非TTY即時return + プロンプト関数をモック可能にする。
- **リスク**: Git状態取得エラーで終了処理が中断する  
  **緩和**: 例外は警告ログに留め、処理継続する。
