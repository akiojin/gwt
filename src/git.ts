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

function getBranchType(branchName: string): BranchInfo['branchType'] {
  if (branchName === 'main' || branchName === 'master') return 'main';
  if (branchName === 'develop' || branchName === 'development') return 'develop';
  if (branchName.startsWith('feature/')) return 'feature';
  if (branchName.startsWith('hotfix/')) return 'hotfix';
  if (branchName.startsWith('release/')) return 'release';
  return 'other';
}