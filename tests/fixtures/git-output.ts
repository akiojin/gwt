/**
 * テスト用のGitコマンド出力データ
 */

/**
 * git branch --list の出力例
 */
export const gitBranchListOutput = `* main
  develop
  feature/user-auth
  feature/dashboard
  hotfix/security-patch
  release/1.2.0`;

/**
 * git branch -r の出力例
 */
export const gitBranchRemoteOutput = `  origin/main
  origin/develop
  origin/feature/api-integration
  origin/hotfix/bug-123`;

/**
 * git worktree list の出力例
 */
export const gitWorktreeListOutput = `/path/to/repo  abc1234 [main]
/path/to/worktree-feature-user-auth  def5678 [feature/user-auth]
/path/to/worktree-feature-dashboard  ghi9012 [feature/dashboard]`;

/**
 * git status の出力例（クリーン）
 */
export const gitStatusCleanOutput = `On branch main
Your branch is up to date with 'origin/main'.

nothing to commit, working tree clean`;

/**
 * git status の出力例（未コミット変更あり）
 */
export const gitStatusDirtyOutput = `On branch feature/user-auth
Your branch is ahead of 'origin/feature/user-auth' by 2 commits.
  (use "git push" to publish your local commits)

Changes not staged for commit:
  (use "git add <file>..." to update what will be committed)
  (use "git restore <file>..." to discard changes in working directory)
	modified:   src/index.ts

Untracked files:
  (use "git add <file>..." to include in what will be committed)
	src/new-feature.ts

no changes added to commit (use "git add" and/or "git commit -a")`;

/**
 * git diff の出力例
 */
export const gitDiffOutput = `diff --git a/src/index.ts b/src/index.ts
index abc1234..def5678 100644
--- a/src/index.ts
+++ b/src/index.ts
@@ -10,6 +10,7 @@ import { someFunction } from './utils';

 async function main() {
   console.log('Starting application...');
+  console.log('New feature added');
   await someFunction();
 }

 main();`;

/**
 * git log の出力例
 */
export const gitLogOutput = `commit abc1234def5678 (HEAD -> feature/user-auth)
Author: Test User <test@example.com>
Date:   Mon Jan 1 10:00:00 2025 +0000

    Add user authentication feature

commit 9012345ghi6789
Author: Test User <test@example.com>
Date:   Sun Dec 31 15:30:00 2024 +0000

    Initial commit`;
