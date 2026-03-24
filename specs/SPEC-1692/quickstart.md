### 基本操作（3ステップ）

1. **ダイアログを開く**: サイドバーでブランチ選択 → Launch ボタン、または メニュー → Launch Agent
2. **設定**: エージェント選択 → モデル選択 → 必要に応じて新規ブランチ/Docker/Advanced 設定
3. **起動**: Launch ボタン → 7ステップ進捗モーダル → エージェント起動完了

### テスト実行

```bash
# 全フロントエンドテスト実行
cd gwt-gui && pnpm test

# 個別テスト実行
cd gwt-gui && pnpm test src/lib/components/AgentLaunchForm.test.ts
cd gwt-gui && pnpm test src/lib/components/agentLaunchFormHelpers.test.ts
cd gwt-gui && pnpm test src/lib/components/agentLaunchDefaults.test.ts

# 型チェック
cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json
```

---
