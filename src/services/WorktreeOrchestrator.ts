import { worktreeExists, generateWorktreePath, createWorktree } from '../worktree.js';

/**
 * WorktreeOrchestrator - Manages worktree existence checks and creation
 *
 * Responsibility:
 * - Check if worktree exists for a given branch
 * - Create worktree if it doesn't exist
 * - Return worktree path
 */
export class WorktreeOrchestrator {
  /**
   * Ensure worktree exists for the given branch
   * If worktree exists, return its path
   * If worktree doesn't exist, create it and return the path
   *
   * @param branch - Branch name
   * @param repoRoot - Repository root path
   * @param baseBranch - Base branch for new worktree (default: 'main')
   * @returns Worktree path
   */
  async ensureWorktree(
    branch: string,
    repoRoot: string,
    baseBranch: string = 'main'
  ): Promise<string> {
    // Check if worktree already exists
    const existingPath = await worktreeExists(branch);

    if (existingPath) {
      return existingPath;
    }

    // Generate worktree path
    const worktreePath = await generateWorktreePath(branch, repoRoot);

    // Create worktree
    await createWorktree({
      branchName: branch,
      worktreePath,
      repoRoot,
      isNewBranch: false,
      baseBranch,
    });

    return worktreePath;
  }
}
