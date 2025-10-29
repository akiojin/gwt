import type { WorktreeConfig } from "../worktree.js";
import {
  worktreeExists,
  generateWorktreePath,
  createWorktree,
} from "../worktree.js";
import { getCurrentBranch } from "../git.js";
import chalk from "chalk";

/**
 * WorktreeService interface for dependency injection
 */
export interface WorktreeService {
  worktreeExists: (branch: string) => Promise<string | null>;
  generateWorktreePath: (repoRoot: string, branch: string) => Promise<string>;
  createWorktree: (config: WorktreeConfig) => Promise<void>;
}

export interface EnsureWorktreeOptions {
  baseBranch?: string;
  isNewBranch?: boolean;
}

/**
 * WorktreeOrchestrator - Manages worktree existence checks and creation
 *
 * Responsibility:
 * - Check if worktree exists for a given branch
 * - Create worktree if it doesn't exist
 * - Return worktree path
 */
export class WorktreeOrchestrator {
  private worktreeService: WorktreeService;

  constructor(worktreeService?: WorktreeService) {
    this.worktreeService = worktreeService || {
      worktreeExists,
      generateWorktreePath,
      createWorktree,
    };
  }

  /**
   * Ensure worktree exists for the given branch
   * If worktree exists, return its path
   * If worktree doesn't exist, create it and return the path
   *
   * @param branch - Branch name
   * @param repoRoot - Repository root path
   * @param options - Creation options (base branch, new branch flag)
   * @returns Worktree path
   */
  async ensureWorktree(
    branch: string,
    repoRoot: string,
    options: EnsureWorktreeOptions = {},
  ): Promise<string> {
    const baseBranch = options.baseBranch ?? "main";
    const isNewBranch = options.isNewBranch ?? false;

    // Check if selected branch is current branch
    const currentBranch = await getCurrentBranch();
    if (currentBranch === branch) {
      // Current branch selected: use repository root
      console.log(
        chalk.gray(
          `   ℹ️  Current branch '${branch}' selected - using repository root`,
        ),
      );
      return repoRoot;
    }

    // Check if worktree already exists
    const existingPath = await this.worktreeService.worktreeExists(branch);

    if (existingPath) {
      return existingPath;
    }

    // Generate worktree path
    const worktreePath = await this.worktreeService.generateWorktreePath(
      repoRoot,
      branch,
    );

    try {
      // Create worktree (or branch)
      await this.worktreeService.createWorktree({
        branchName: branch,
        worktreePath,
        repoRoot,
        isNewBranch,
        baseBranch,
      });

      return worktreePath;
    } catch (error: unknown) {
      const message =
        error instanceof Error ? error.message : String(error ?? "");
      const normalized = message.toLowerCase();
      const alreadyExists =
        normalized.includes("already checked out") ||
        normalized.includes("already exists");

      if (alreadyExists) {
        const fallbackPath = await this.worktreeService.worktreeExists(branch);
        if (fallbackPath) {
          return fallbackPath;
        }
      }

      throw error;
    }
  }
}
