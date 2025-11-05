import type {
  BatchMergeConfig,
  BatchMergeProgress,
  BatchMergeResult,
  BranchMergeStatus,
  BatchMergeSummary,
} from "../ui/types";
import * as git from "../git";
import * as worktree from "../worktree";

/**
 * BatchMergeService - Orchestrates batch merge operations
 * @see specs/SPEC-ee33ca26/plan.md - Service layer architecture
 */
export class BatchMergeService {
  /**
   * Determine source branch for merge (main > develop > master)
   * @returns Source branch name
   * @throws Error if no source branch found
   * @see specs/SPEC-ee33ca26/spec.md - FR-004
   */
  async determineSourceBranch(): Promise<string> {
    const branches = await git.getLocalBranches();
    const branchNames = branches.map((b) => b.name);

    // Priority: main > develop > master
    if (branchNames.includes("main")) {
      return "main";
    }
    if (branchNames.includes("develop")) {
      return "develop";
    }
    if (branchNames.includes("master")) {
      return "master";
    }

    throw new Error("マージ元ブランチを特定できません");
  }

  /**
   * Get target branches for merge (exclude main, develop, master)
   * @returns Array of target branch names
   * @see specs/SPEC-ee33ca26/spec.md - FR-003
   */
  async getTargetBranches(): Promise<string[]> {
    const branches = await git.getLocalBranches();
    const excludedBranches = ["main", "develop", "master"];

    return branches
      .map((b) => b.name)
      .filter((name) => !excludedBranches.includes(name));
  }

  /**
   * Ensure worktree exists for target branch
   * @param branchName - Target branch name
   * @returns Worktree path
   * @see specs/SPEC-ee33ca26/spec.md - FR-006
   */
  async ensureWorktree(branchName: string): Promise<string> {
    const worktrees = await worktree.listAdditionalWorktrees();
    const existingWorktree = worktrees.find((w: { path: string }) =>
      w.path.includes(branchName.replace(/\//g, "-")),
    );

    if (existingWorktree) {
      return existingWorktree.path;
    }

    // Create new worktree
    const repoRoot = await git.getRepositoryRoot();
    const worktreePath = await worktree.generateWorktreePath(
      repoRoot,
      branchName,
    );
    await worktree.createWorktree({
      branchName,
      worktreePath,
      repoRoot,
      isNewBranch: false,
      baseBranch: "",
    });
    return worktreePath;
  }

  /**
   * Merge source branch into target branch
   * @param branchName - Target branch name
   * @param sourceBranch - Source branch name
   * @param config - Batch merge configuration
   * @returns Merge status for the branch
   * @see specs/SPEC-ee33ca26/spec.md - FR-007, FR-008
   */
  async mergeBranch(
    branchName: string,
    sourceBranch: string,
    config: BatchMergeConfig,
  ): Promise<BranchMergeStatus> {
    const startTime = Date.now();
    let worktreeCreated = false;

    try {
      // Ensure worktree exists
      const worktrees = await worktree.listAdditionalWorktrees();
      const worktreePath = await this.ensureWorktree(branchName);
      worktreeCreated = !worktrees.some(
        (w: { path: string }) => w.path === worktreePath,
      );

      // Execute merge
      await git.mergeFromBranch(worktreePath, sourceBranch, config.dryRun);

      // Check for conflicts
      const hasConflict = await git.hasMergeConflict(worktreePath);

      if (hasConflict) {
        await git.abortMerge(worktreePath);
        return {
          branchName,
          status: "skipped",
          worktreeCreated,
          durationSeconds: (Date.now() - startTime) / 1000,
        };
      }

      // Rollback dry-run merge
      if (config.dryRun) {
        await git.resetToHead(worktreePath);
      }

      // Auto-push after successful merge
      let pushStatus: "success" | "failed" | "not_executed" = "not_executed";
      if (config.autoPush && !config.dryRun) {
        try {
          const currentBranch = await git.getCurrentBranchName(worktreePath);
          await git.pushBranchToRemote(
            worktreePath,
            currentBranch,
            config.remote || "origin",
          );
          pushStatus = "success";
        } catch {
          // Push failure should not fail the merge
          pushStatus = "failed";
        }
      }

      return {
        branchName,
        status: "success",
        pushStatus,
        worktreeCreated,
        durationSeconds: (Date.now() - startTime) / 1000,
      };
    } catch (error) {
      // Check if it's a merge conflict
      const hasConflict = await git.hasMergeConflict(
        await this.ensureWorktree(branchName),
      );

      if (hasConflict) {
        try {
          await git.abortMerge(await this.ensureWorktree(branchName));
        } catch {
          // Ignore abort errors
        }

        return {
          branchName,
          status: "skipped",
          worktreeCreated,
          durationSeconds: (Date.now() - startTime) / 1000,
        };
      }

      return {
        branchName,
        status: "failed",
        error: error instanceof Error ? error.message : String(error),
        worktreeCreated,
        durationSeconds: (Date.now() - startTime) / 1000,
      };
    }
  }

  /**
   * Execute batch merge for all target branches
   * @param config - Batch merge configuration
   * @param onProgress - Progress callback function
   * @returns Batch merge result
   * @see specs/SPEC-ee33ca26/spec.md - FR-001 to FR-015
   */
  async executeBatchMerge(
    config: BatchMergeConfig,
    onProgress?: (progress: BatchMergeProgress) => void,
  ): Promise<BatchMergeResult> {
    const startTime = Date.now();
    const statuses: BranchMergeStatus[] = [];

    // Fetch latest from remote
    await git.fetchAllRemotes();

    const totalBranches = config.targetBranches.length;

    for (let i = 0; i < totalBranches; i++) {
      const branchName = config.targetBranches[i] || "";
      const elapsedSeconds = (Date.now() - startTime) / 1000;

      // Report progress
      if (onProgress) {
        const progress: BatchMergeProgress = {
          currentBranch: branchName,
          currentIndex: i,
          totalBranches,
          percentage: Math.floor((i / totalBranches) * 100),
          elapsedSeconds,
          currentPhase: "merge",
        };
        onProgress(progress);
      }

      // Merge branch
      if (branchName) {
        const status = await this.mergeBranch(
          branchName,
          config.sourceBranch,
          config,
        );
        statuses.push(status);
      }
    }

    // Calculate summary
    const summary: BatchMergeSummary = {
      totalCount: statuses.length,
      successCount: statuses.filter((s) => s.status === "success").length,
      skippedCount: statuses.filter((s) => s.status === "skipped").length,
      failedCount: statuses.filter((s) => s.status === "failed").length,
      pushedCount: 0,
      pushFailedCount: 0,
    };

    return {
      statuses,
      summary,
      totalDurationSeconds: (Date.now() - startTime) / 1000,
      cancelled: false,
      config,
    };
  }
}
