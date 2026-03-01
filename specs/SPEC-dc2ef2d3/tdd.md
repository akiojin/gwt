# TDD: vi 終了時の docs 編集タブ自動クローズ

## 対象

- `gwt-gui/src/lib/docsEditor.ts`
- `gwt-gui/src/App.svelte`（`docsEditor.ts` を利用）

## RED

1. `gwt-gui/src/lib/docsEditor.test.ts` を追加。
2. 先にテストを実行して、`docsEditor` モジュール未解決で失敗を確認。

実行コマンド:

```bash
pnpm --dir gwt-gui test -- src/lib/docsEditor.test.ts
```

失敗要約:

- `Failed to resolve import "./docsEditor" from "src/lib/docsEditor.test.ts"`

## GREEN

1. `gwt-gui/src/lib/docsEditor.ts` を実装。
2. `App.svelte` の command 生成/終了判定ロジックを `docsEditor.ts` へ移管。
3. `vi` 経路を `vi ...; exit` に統一。

実行コマンド:

```bash
pnpm --dir gwt-gui exec vitest run src/lib/docsEditor.test.ts
```

結果:

- `7 passed`

## 回帰確認

実行コマンド:

```bash
pnpm --dir gwt-gui check
```

結果:

- `0 errors`（既存 warning 1 件のみ）
