import { execa } from 'execa';
import { BranchInfo } from './ui/types.js';

export class GitError extends Error {
  constructor(message: string, public cause?: unknown) {
    super(message);
    this.name = 'GitError';
  }
}

/**
 * 現在のディレクトリがGitリポジトリかどうかを確認
 * @returns {Promise<boolean>} Gitリポジトリの場合true
 */
export async function isGitRepository(): Promise<boolean> {
  try {
    await execa('git', ['rev-parse', '--git-dir']);
    return true;
  } catch {
    return false;
  }
}

/**
 * Gitリポジトリのルートディレクトリを取得
 * @returns {Promise<string>} リポジトリのルートパス
 * @throws {GitError} リポジトリルートの取得に失敗した場合
 */
export async function getRepositoryRoot(): Promise<string> {
  try {
    const { stdout } = await execa('git', ['rev-parse', '--show-toplevel']);
    return stdout.trim();
  } catch (error) {
    throw new GitError('Failed to get repository root', error);
  }
}



export async function getRemoteBranches(): Promise<BranchInfo[]> {
  try {
    const { stdout } = await execa('git', ['branch', '-r', '--format=%(refname:short)']);
    return stdout
      .split('\n')
      .filter(line => line.trim() && !line.includes('HEAD'))
      .map(line => {
        const name = line.trim();
        const branchName = name.replace(/^origin\//, '');
        return {
          name,
          type: 'remote' as const,
          branchType: getBranchType(branchName),
          isCurrent: false
        };
      });
  } catch (error) {
    throw new GitError('Failed to get remote branches', error);
  }
}

async function getCurrentBranch(): Promise<string | null> {
  try {
    const { stdout } = await execa('git', ['branch', '--show-current']);
    return stdout.trim() || null;
  } catch {
    return null;
  }
}

export async function getLocalBranches(): Promise<BranchInfo[]> {
  try {
    const { stdout } = await execa('git', ['branch', '--format=%(refname:short)']);
    return stdout
      .split('\n')
      .filter(line => line.trim())
      .map(name => ({
        name: name.trim(),
        type: 'local' as const,
        branchType: getBranchType(name.trim()),
        isCurrent: false
      }));
  } catch (error) {
    throw new GitError('Failed to get local branches', error);
  }
}

/**
 * ローカルとリモートのすべてのブランチ情報を取得
 * @returns {Promise<BranchInfo[]>} ブランチ情報の配列
 */
export async function getAllBranches(): Promise<BranchInfo[]> {
  const [localBranches, remoteBranches, currentBranch] = await Promise.all([
    getLocalBranches(),
    getRemoteBranches(),
    getCurrentBranch()
  ]);
  
  // 現在のブランチ情報を設定
  if (currentBranch) {
    localBranches.forEach(branch => {
      if (branch.name === currentBranch) {
        branch.isCurrent = true;
      }
    });
  }
  
  return [...localBranches, ...remoteBranches];
}

export async function createBranch(branchName: string, baseBranch = 'main'): Promise<void> {
  try {
    await execa('git', ['checkout', '-b', branchName, baseBranch]);
  } catch (error) {
    throw new GitError(`Failed to create branch ${branchName}`, error);
  }
}

export async function branchExists(branchName: string): Promise<boolean> {
  try {
    await execa('git', ['show-ref', '--verify', '--quiet', `refs/heads/${branchName}`]);
    return true;
  } catch {
    return false;
  }
}

export async function deleteBranch(branchName: string, force = false): Promise<void> {
  try {
    const args = ['branch', force ? '-D' : '-d', branchName];
    await execa('git', args);
  } catch (error) {
    throw new GitError(`Failed to delete branch ${branchName}`, error);
  }
}


interface WorktreeStatusResult {
  hasChanges: boolean;
  changedFilesCount: number;
}

async function getWorkdirStatus(worktreePath: string): Promise<WorktreeStatusResult> {
  try {
    const { stdout } = await execa('git', ['status', '--porcelain'], { cwd: worktreePath });
    const lines = stdout.split('\n').filter(line => line.trim());
    return {
      hasChanges: lines.length > 0,
      changedFilesCount: lines.length
    };
  } catch (error) {
    throw new GitError('Failed to get worktree status', error);
  }
}

export async function hasUncommittedChanges(worktreePath: string): Promise<boolean> {
  const status = await getWorkdirStatus(worktreePath);
  return status.hasChanges;
}

export async function getChangedFilesCount(worktreePath: string): Promise<number> {
  const status = await getWorkdirStatus(worktreePath);
  return status.changedFilesCount;
}

export async function showStatus(worktreePath: string): Promise<string> {
  try {
    const { stdout } = await execa('git', ['status'], { cwd: worktreePath });
    return stdout;
  } catch (error) {
    throw new GitError('Failed to show status', error);
  }
}

export async function stashChanges(worktreePath: string, message?: string): Promise<void> {
  try {
    const args = message ? ['stash', 'push', '-m', message] : ['stash'];
    await execa('git', args, { cwd: worktreePath });
  } catch (error) {
    throw new GitError('Failed to stash changes', error);
  }
}

export async function discardAllChanges(worktreePath: string): Promise<void> {
  try {
    // Reset tracked files
    await execa('git', ['reset', '--hard'], { cwd: worktreePath });
    // Clean untracked files
    await execa('git', ['clean', '-fd'], { cwd: worktreePath });
  } catch (error) {
    throw new GitError('Failed to discard changes', error);
  }
}

export async function commitChanges(worktreePath: string, message: string): Promise<void> {
  try {
    // Add all changes
    await execa('git', ['add', '-A'], { cwd: worktreePath });
    // Commit
    await execa('git', ['commit', '-m', message], { cwd: worktreePath });
  } catch (error) {
    throw new GitError('Failed to commit changes', error);
  }
}

function getBranchType(branchName: string): BranchInfo['branchType'] {
  if (branchName === 'main' || branchName === 'master') return 'main';
  if (branchName === 'develop' || branchName === 'dev') return 'develop';
  if (branchName.startsWith('feature/')) return 'feature';
  if (branchName.startsWith('hotfix/')) return 'hotfix';
  if (branchName.startsWith('release/')) return 'release';
  return 'other';
}

export async function hasUnpushedCommits(worktreePath: string, branch: string): Promise<boolean> {
  try {
    const { stdout } = await execa('git', ['log', `origin/${branch}..${branch}`, '--oneline'], { cwd: worktreePath });
    return stdout.trim().length > 0;
  } catch {
    // If the branch doesn't exist on remote, consider it has unpushed commits
    return true;
  }
}


export async function fetchAllRemotes(): Promise<void> {
  try {
    await execa('git', ['fetch', '--all', '--prune']);
  } catch (error) {
    throw new GitError('Failed to fetch remote branches', error);
  }
}

export async function deleteRemoteBranch(branchName: string, remote = 'origin'): Promise<void> {
  try {
    await execa('git', ['push', remote, '--delete', branchName]);
  } catch (error) {
    throw new GitError(`Failed to delete remote branch ${remote}/${branchName}`, error);
  }
}

export async function pushBranchToRemote(worktreePath: string, branchName: string, remote = 'origin'): Promise<void> {
  try {
    // Check if the remote branch exists
    const remoteBranchExists = await checkRemoteBranchExists(branchName, remote);
    
    if (remoteBranchExists) {
      // Push to existing remote branch
      await execa('git', ['push', remote, branchName], { cwd: worktreePath });
    } else {
      // Push and set upstream for new remote branch
      await execa('git', ['push', '--set-upstream', remote, branchName], { cwd: worktreePath });
    }
  } catch (error) {
    throw new GitError(`Failed to push branch ${branchName} to ${remote}`, error);
  }
}

export async function checkRemoteBranchExists(branchName: string, remote = 'origin'): Promise<boolean> {
  try {
    await execa('git', ['show-ref', '--verify', '--quiet', `refs/remotes/${remote}/${branchName}`]);
    return true;
  } catch {
    return false;
  }
}