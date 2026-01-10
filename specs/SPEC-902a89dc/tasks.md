---
description: "Worktreeパス修復機能の対象拡張タスク"
---

# タスク: Worktreeパス修復機能（チェック済み対象拡張）

**仕様ID**: `SPEC-902a89dc`
**入力**: `specs/SPEC-902a89dc/spec.md` / `specs/SPEC-902a89dc/plan.md`

## フェーズ1: ユーザーストーリー2 - チェック済みブランチの修復実行 (優先度: P1)

**ストーリー**: チェック済みローカルブランチをアクセス可能/不可能に関わらず修復対象に含める。

**価値**: ユーザーが選んだ対象を確実に修復できる。

### テスト（TDD）

- [x] **T101** [US2] `src/cli/ui/__tests__/solid/AppSolid.cleanup.test.tsx` にアクセス可能なWorktreeが修復対象になるテストを追加

### 実装

- [x] **T102** [US2] `src/cli/ui/App.solid.tsx` の修復対象判定を更新し、アクセス可否による除外を撤廃

## フェーズ2: 統合確認

- [x] **T201** [統合] `bun test src/cli/ui/__tests__/solid/AppSolid.cleanup.test.tsx` を実行し、新規テストを含めてパスすることを確認
