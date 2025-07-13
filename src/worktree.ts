import { execa } from 'execa';
import path from 'path';
import { WorktreeConfig, WorktreeWithPR, CleanupTarget } from './ui/types.js';
import { getPullRequestByBranch, getMergedPullRequests } from './github.js';
import { hasUncommittedChanges, hasUnpushedCommits } from './git.js';

export class WorktreeError extends Error {
  constructor(message: string, public cause?: unknown) {
    super(message);
    this.name = 'WorktreeError';
  }
}

export interface WorktreeInfo {
  path: string;
  branch: string;
  head: string;
}

export async function listWorktrees(): Promise<WorktreeInfo[]> {
  try {
    const { stdout } = await execa('git', ['worktree', 'list', '--porcelain']);
    const worktrees: WorktreeInfo[] = [];
    const lines = stdout.split('\n');
    
    let currentWorktree: Partial<WorktreeInfo> = {};
    
    for (const line of lines) {
      if (line.startsWith('worktree ')) {
        if (currentWorktree.path) {
          worktrees.push(currentWorktree as WorktreeInfo);
        }
        currentWorktree = { path: line.substring(9) };
      } else if (line.startsWith('HEAD ')) {
        currentWorktree.head = line.substring(5);
      } else if (line.startsWith('branch ')) {
        currentWorktree.branch = line.substring(7).replace('refs/heads/', '');
      } else if (line === '') {
        if (currentWorktree.path) {
          worktrees.push(currentWorktree as WorktreeInfo);
          currentWorktree = {};
        }
      }
    }
    
    if (currentWorktree.path) {
      worktrees.push(currentWorktree as WorktreeInfo);
    }
    
    return worktrees;
  } catch (error) {
    throw new WorktreeError('Failed to list worktrees', error);
  }
}

export async function listAdditionalWorktrees(): Promise<WorktreeInfo[]> {
  try {
    const [allWorktrees, repoRoot] = await Promise.all([
      listWorktrees(),
      import('./git.js').then(m => m.getRepositoryRoot())
    ]);
    
    // Filter out the main worktree (repository root)
    return allWorktrees.filter(worktree => worktree.path !== repoRoot);
  } catch (error) {
    throw new WorktreeError('Failed to list additional worktrees', error);
  }
}

export async function worktreeExists(branchName: string): Promise<string | null> {
  const worktrees = await listWorktrees();
  const worktree = worktrees.find(w => w.branch === branchName);
  return worktree ? worktree.path : null;
}

export async function generateWorktreePath(repoRoot: string, branchName: string): Promise<string> {
  const sanitizedBranchName = branchName.replace(/[\/\\:*?"<>|]/g, '-');
  const worktreeDir = path.join(repoRoot, '.git', 'worktree');
  return path.join(worktreeDir, sanitizedBranchName);
}

export async function createWorktree(config: WorktreeConfig): Promise<void> {
  try {
    const args = ['worktree', 'add'];
    
    if (config.isNewBranch) {
      args.push('-b', config.branchName);
    }
    
    args.push(config.worktreePath);
    
    if (config.isNewBranch) {
      args.push(config.baseBranch);
    } else {
      args.push(config.branchName);
    }
    
    await execa('git', args);
  } catch (error) {
    throw new WorktreeError(`Failed to create worktree for ${config.branchName}`, error);
  }
}

export async function removeWorktree(worktreePath: string, force: boolean = false): Promise<void> {
  try {
    const args = ['worktree', 'remove'];
    if (force) {
      args.push('--force');
    }
    args.push(worktreePath);
    
    await execa('git', args);
  } catch (error) {
    throw new WorktreeError(`Failed to remove worktree at ${worktreePath}`, error);
  }
}

export async function pruneWorktrees(): Promise<void> {
  try {
    await execa('git', ['worktree', 'prune']);
  } catch (error) {
    throw new WorktreeError('Failed to prune worktrees', error);
  }
}

export async function getWorktreesWithPRStatus(): Promise<WorktreeWithPR[]> {
  const worktrees = await listAdditionalWorktrees();
  const worktreesWithPR: WorktreeWithPR[] = [];
  
  for (const worktree of worktrees) {
    if (worktree.branch) {
      const pullRequest = await getPullRequestByBranch(worktree.branch);
      worktreesWithPR.push({
        worktreePath: worktree.path,
        branch: worktree.branch,
        pullRequest
      });
    }
  }
  
  return worktreesWithPR;
}

export async function getMergedPRWorktrees(): Promise<CleanupTarget[]> {
  const worktreesWithPR = await getWorktreesWithPRStatus();
  const mergedPRs = await getMergedPullRequests();
  const cleanupTargets: CleanupTarget[] = [];
  
  for (const worktree of worktreesWithPR) {
    const mergedPR = mergedPRs.find(pr => pr.branch === worktree.branch);
    if (mergedPR) {
      const hasUncommitted = await hasUncommittedChanges(worktree.worktreePath);
      const hasUnpushed = await hasUnpushedCommits(worktree.worktreePath, worktree.branch);
      
      cleanupTargets.push({
        worktreePath: worktree.worktreePath,
        branch: worktree.branch,
        pullRequest: mergedPR,
        hasUncommittedChanges: hasUncommitted,
        hasUnpushedCommits: hasUnpushed
      });
    }
  }
  
  return cleanupTargets;
}