import { execa } from 'execa';
import { BranchInfo } from './ui/types.js';

export class GitError extends Error {
  constructor(message: string, public cause?: unknown) {
    super(message);
    this.name = 'GitError';
  }
}

export async function isGitRepository(): Promise<boolean> {
  try {
    await execa('git', ['rev-parse', '--git-dir']);
    return true;
  } catch {
    return false;
  }
}

export async function getRepositoryRoot(): Promise<string> {
  try {
    const { stdout } = await execa('git', ['rev-parse', '--show-toplevel']);
    return stdout.trim();
  } catch (error) {
    throw new GitError('Failed to get repository root', error);
  }
}

export async function getCurrentBranch(): Promise<string> {
  try {
    const { stdout } = await execa('git', ['branch', '--show-current']);
    return stdout.trim();
  } catch (error) {
    throw new GitError('Failed to get current branch', error);
  }
}

export async function getLocalBranches(): Promise<BranchInfo[]> {
  try {
    const { stdout } = await execa('git', ['branch', '--format=%(refname:short)|%(HEAD)']);
    return stdout
      .split('\n')
      .filter(line => line.trim())
      .map(line => {
        const [name, isHead] = line.split('|');
        const branchName = name?.trim() ?? '';
        return {
          name: branchName,
          type: 'local' as const,
          branchType: getBranchType(branchName),
          isCurrent: isHead === '*'
        };
      });
  } catch (error) {
    throw new GitError('Failed to get local branches', error);
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

export async function getAllBranches(): Promise<BranchInfo[]> {
  const [localBranches, remoteBranches] = await Promise.all([
    getLocalBranches(),
    getRemoteBranches()
  ]);
  
  return [...localBranches, ...remoteBranches];
}

export async function createBranch(branchName: string, baseBranch: string = 'main'): Promise<void> {
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

export async function deleteBranch(branchName: string, force: boolean = false): Promise<void> {
  try {
    const args = ['branch', force ? '-D' : '-d', branchName];
    await execa('git', args);
  } catch (error) {
    throw new GitError(`Failed to delete branch ${branchName}`, error);
  }
}

export interface WorktreeStatus {
  hasChanges: boolean;
  changedFiles: number;
  stagedFiles: number;
  untrackedFiles: number;
}

export async function getWorktreeStatus(worktreePath: string): Promise<WorktreeStatus> {
  try {
    const { stdout } = await execa('git', ['status', '--porcelain'], { cwd: worktreePath });
    const lines = stdout.split('\n').filter(line => line.trim());
    
    let stagedFiles = 0;
    let changedFiles = 0;
    let untrackedFiles = 0;
    
    for (const line of lines) {
      const status = line.substring(0, 2);
      if (status.includes('?')) {
        untrackedFiles++;
      } else if (status[0] !== ' ' && status[0] !== '?') {
        stagedFiles++;
      } else if (status[1] !== ' ' && status[1] !== '?') {
        changedFiles++;
      }
    }
    
    return {
      hasChanges: lines.length > 0,
      changedFiles: lines.length,
      stagedFiles,
      untrackedFiles
    };
  } catch (error) {
    throw new GitError('Failed to get worktree status', error);
  }
}

export async function hasUncommittedChanges(worktreePath: string): Promise<boolean> {
  try {
    const { stdout } = await execa('git', ['status', '--porcelain'], { cwd: worktreePath });
    return stdout.trim().length > 0;
  } catch (error) {
    throw new GitError('Failed to check uncommitted changes', error);
  }
}

export async function getChangedFilesCount(worktreePath: string): Promise<number> {
  try {
    const { stdout } = await execa('git', ['status', '--porcelain'], { cwd: worktreePath });
    return stdout.split('\n').filter(line => line.trim()).length;
  } catch (error) {
    throw new GitError('Failed to get changed files count', error);
  }
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
  if (branchName === 'develop' || branchName === 'development') return 'develop';
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

export async function isBranchMerged(branch: string, targetBranch: string = 'main'): Promise<boolean> {
  try {
    const { stdout } = await execa('git', ['branch', '--merged', targetBranch]);
    return stdout.includes(branch);
  } catch (error) {
    throw new GitError(`Failed to check if branch ${branch} is merged`, error);
  }
}

export async function fetchAllRemotes(): Promise<void> {
  try {
    await execa('git', ['fetch', '--all', '--prune']);
  } catch (error) {
    throw new GitError('Failed to fetch remote branches', error);
  }
}

export async function deleteRemoteBranch(branchName: string, remote: string = 'origin'): Promise<void> {
  try {
    await execa('git', ['push', remote, '--delete', branchName]);
  } catch (error) {
    throw new GitError(`Failed to delete remote branch ${remote}/${branchName}`, error);
  }
}