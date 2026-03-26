### Phase 1: SPEC 策定（本 Issue 作成）— Complete

- 現行実装の分析と文書化
- CLOSED SPECs (#1317, #1306, #1337) の統合
- FR-001〜FR-048 の定義

### Phase 2: FR-048 実装 — Complete

**Technical context:**
- Frontend: `gwt-gui/src/lib/components/AgentLaunchForm.svelte`（プレフィックスドロップダウン削除）
- Frontend: `gwt-gui/src/lib/components/agentLaunchFormHelpers.ts`（`classifyIssuePrefix()` フォールバック変更）
- Tests: `gwt-gui/src/lib/components/agentLaunchFormHelpers.test.ts`（新規テスト追加）
- Tests: `gwt-gui/src/lib/components/AgentLaunchForm.test.ts`（新規テスト追加）

**Implementation approach:**
1. テストファースト: `agentLaunchFormHelpers.test.ts`, `AgentLaunchForm.test.ts` に FR-048 用テスト追加（RED 確認）
2. `agentLaunchFormHelpers.ts`: `classifyIssuePrefix()` の空文字フォールバック → `"feature/"` に変更
3. `AgentLaunchForm.svelte`: プレフィックスドロップダウン UI 削除、分類中インジケーター表示の簡素化、AI 失敗時ハンドリング変更
4. テスト GREEN 確認
5. 型チェック・lint 通過確認

---
