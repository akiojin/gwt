### Phase 1: 現行実装との同期
- PR #1617 以降の overlay 実装を確認し、spec と現行 behavior の差分を整理する
- overlay Paste の役割を text paste ではなく image staging に同期する

### Phase 2: TDD で visibility contract を固定
- `TerminalView.test.ts` に icon size / button size / contrast / pointer-events contract を表すテストを追加する
- 現状 16px icon の RED を確認してから実装に進む

### Phase 3: 視認性改善の実装と検証
- `TerminalView.svelte` の icon size / min-size / padding / gap / contrast を更新する
- `pnpm exec vitest run src/lib/terminal/TerminalView.test.ts` と `pnpm exec svelte-check --tsconfig ./tsconfig.json` で検証する

---
