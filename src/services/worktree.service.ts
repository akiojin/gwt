import path from "node:path";
import { WorktreeRepository } from "../repositories/worktree.repository.js";
import { GitRepository } from "../repositories/git.repository.js";
import { WorktreeInfo } from "../worktree.js";
import { WorktreeConfig } from "../cli/ui/types.js";

/**
 * Worktree操作のビジネスロジックを管理するService
 */
export class WorktreeService {
  constructor(
    private readonly worktreeRepository: WorktreeRepository,
    private readonly gitRepository: GitRepository,
  ) {}

  async listAdditionalWorktrees(): Promise<WorktreeInfo[]> {
    const [allWorktrees, repoRoot] = await Promise.all([
      this.listAllWorktrees(),
      this.gitRepository.getRepositoryRoot(),
    ]);

    // メインworktree（リポジトリルート）を除外
    return allWorktrees.filter((w) => w.path !== repoRoot);
  }

  async listAllWorktrees(): Promise<WorktreeInfo[]> {
    const worktrees = await this.worktreeRepository.list();
    return worktrees.map((w) => ({
      path: w.path,
      branch: w.branch,
      head: w.head,
    }));
  }

  async createWorktree(config: WorktreeConfig): Promise<void> {
    await this.worktreeRepository.add(config.worktreePath, config.branchName);
  }

  async removeWorktree(worktreePath: string, force = false): Promise<void> {
    await this.worktreeRepository.remove(worktreePath, force);
  }

  async getWorktreeByBranch(
    branchName: string,
  ): Promise<WorktreeInfo | undefined> {
    const worktrees = await this.listAllWorktrees();
    return worktrees.find((w) => w.branch === branchName);
  }

  async getRecommendedWorktreePath(branchName: string): Promise<string> {
    const repoRoot = await this.gitRepository.getRepositoryRoot();
    const repoName = path.basename(repoRoot);
    const parentDir = path.dirname(repoRoot);

    // ブランチ名からworktreeパスを生成
    const safeBranchName = branchName
      .replace(/\//g, "-")
      .replace(/[^a-zA-Z0-9-_]/g, "");

    return path.join(parentDir, `${repoName}-${safeBranchName}`);
  }

  async prune(): Promise<void> {
    await this.worktreeRepository.prune();
  }
}
