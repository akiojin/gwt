import { GitHubRepository } from "../repositories/github.repository.js";
import type { PullRequest, MergedPullRequest } from "../ui/types.js";

/**
 * GitHub操作のビジネスロジックを管理するService
 */
export class GitHubService {
  constructor(private readonly repository: GitHubRepository) {}

  async isAvailable(): Promise<boolean> {
    return await this.repository.isAvailable();
  }

  async checkAuthentication(): Promise<boolean> {
    return await this.repository.isAuthenticated();
  }

  async getMergedPullRequests(): Promise<MergedPullRequest[]> {
    // リモート情報を更新
    await this.repository.fetchRemoteUpdates();

    const prs = await this.repository.fetchPullRequests({
      state: "merged",
      limit: 100,
    });

    return prs
      .filter((pr) => pr.mergedAt !== null)
      .map((pr) => ({
        number: pr.number,
        title: pr.title,
        branch: pr.headRefName,
        mergedAt: pr.mergedAt as string, // filterで null を除外済み
        author: pr.author?.login || "unknown",
      }));
  }

  async getPullRequestByBranch(
    branchName: string,
  ): Promise<PullRequest | null> {
    const prs = await this.repository.fetchPullRequests({
      head: branchName,
      state: "all",
      limit: 1,
    });

    if (prs.length === 0 || !prs[0]) {
      return null;
    }

    const pr = prs[0];
    return {
      number: pr.number,
      title: pr.title,
      state: pr.state as "OPEN" | "CLOSED" | "MERGED",
      branch: pr.headRefName,
      mergedAt: pr.mergedAt,
      author: pr.author?.login || "unknown",
    };
  }
}
