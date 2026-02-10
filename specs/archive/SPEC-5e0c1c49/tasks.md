# タスク: Codex CLI gpt-5.1 デフォルト更新

**仕様ID**: `SPEC-5e0c1c49`
**ポリシー**: CLAUDE.md の TDD ルールに基づき、必ず RED→GREEN→リグレッションチェックの順に進める。

## フェーズ1: RED

- [x] **T2001** `tests/unit/codex.test.ts` の `DEFAULT_CODEX_ARGS` と期待配列を `--model=gpt-5.1-codex` に更新し、`bun test tests/unit/codex.test.ts` で失敗を確認する。

## フェーズ2: GREEN

- [x] **T2002** `src/codex.ts` の `DEFAULT_CODEX_ARGS` を同じ文字列に置換し、再度テストを実行して成功させる。

## フェーズ3: リグレッションチェック

- [x] **T2003** `rg "gpt-5-codex" src tests README.md README.ja.md` を実行して旧モデル名が残っていないことを確認し、`bun test` の結果をログへ記録する。
