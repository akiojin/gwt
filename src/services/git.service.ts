import { GitRepository } from '../repositories/git.repository.js';
import { BranchInfo } from '../ui/types.js';

/**
 * Git操作のビジネスロジックを管理するService
 */
export class GitService {
  constructor(private readonly repository: GitRepository) {}

  async isValidRepository(): Promise<boolean> {
    return await this.repository.isRepository();
  }

  async getAllBranches(): Promise<BranchInfo[]> {
    const [localBranches, remoteBranches] = await Promise.all([
      this.getLocalBranchesInfo(),
      this.getRemoteBranchesInfo()
    ]);
    
    return [...localBranches, ...remoteBranches];
  }

  private async getLocalBranchesInfo(): Promise<BranchInfo[]> {
    const branches = await this.repository.getBranches({ remote: false });
    return branches.map(name => ({
      name,
      type: 'local' as const,
      branchType: this.getBranchType(name),
      isCurrent: false // TODO: 現在のブランチ情報を取得
    }));
  }

  private async getRemoteBranchesInfo(): Promise<BranchInfo[]> {
    const branches = await this.repository.getBranches({ remote: true });
    return branches.map(name => ({
      name,
      type: 'remote' as const,
      branchType: this.getBranchType(name.replace(/^origin\//, '')),
      isCurrent: false
    }));
  }

  private getBranchType(branchName: string): BranchInfo['branchType'] {
    if (branchName.startsWith('feature/')) return 'feature';
    if (branchName.startsWith('hotfix/')) return 'hotfix';
    if (branchName.startsWith('release/')) return 'release';
    if (branchName === 'main' || branchName === 'master') return 'main';
    if (branchName === 'develop' || branchName === 'dev') return 'develop';
    return 'other';
  }

  async createFeatureBranch(taskName: string, baseBranch: string): Promise<string> {
    const branchName = `feature/${taskName}`;
    await this.repository.createBranch(branchName, baseBranch);
    return branchName;
  }

  async deleteBranch(branchName: string, options?: { 
    force?: boolean; 
    remote?: boolean;
  }): Promise<void> {
    if (options?.remote) {
      await this.repository.deleteRemoteBranch(branchName);
    } else {
      await this.repository.deleteBranch(branchName, options?.force);
    }
  }

  async hasUncommittedChanges(workdir?: string): Promise<boolean> {
    return await this.repository.hasChanges(workdir);
  }

  async getChangedFilesCount(workdir?: string): Promise<number> {
    return await this.repository.getChangedFilesCount(workdir);
  }

  async commitAllChanges(message: string): Promise<void> {
    await this.repository.add('.');
    await this.repository.commit(message, { all: true });
  }

  async pushChanges(branchName: string): Promise<void> {
    await this.repository.push({ upstream: true, branch: branchName });
  }

  async stashChanges(message?: string): Promise<void> {
    await this.repository.stash(message);
  }

  async discardAllChanges(): Promise<void> {
    await this.repository.checkout('.');
  }

  async fetchRemoteUpdates(): Promise<void> {
    await this.repository.fetch({ all: true, prune: true });
  }
}