# クイックスタート: Worktree側での.gitignore更新と重複登録防止

1. **依存インストール**
   ```bash
   bun install
   ```
2. **テスト実行**
   ```bash
   bun test git worktree
   ```
   - 追加予定のユニットテスト: `tests/unit/git.test.ts` と `tests/unit/worktree.test.ts`
3. **動作確認手順（手動）**
   ```bash
   # worktree内で実行
   claude-worktree
   git status   # worktree側のみがクリーンか確認

   # ルートブランチで確認
   cd /path/to/repo
   git status   # .gitignore が変更されていないこと
   ```
4. **Lint/Format**
   ```bash
   bun run lint
   bun run format:check
   bunx --bun markdownlint-cli "**/*.md" --config .markdownlint.json --ignore-path .markdownlintignore
   ```

> すべてのコマンドは worktree ディレクトリ（例: `.worktrees/hotfix-gitignore-worktree`）を起点に実行する。
