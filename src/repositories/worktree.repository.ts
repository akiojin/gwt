import { execa } from 'execa';
import { WorktreeError } from '../worktree.js';

export interface WorktreeData {
  path: string;
  head: string;
  branch: string;
}

/**
 * Git Worktree操作のための低レベルRepository
 */
export class WorktreeRepository {
  async execute(args: string[], options?: { cwd?: string }): Promise<string> {
    try {
      const { stdout } = await execa('git', ['worktree', ...args], options);
      return stdout;
    } catch (error) {
      throw new WorktreeError(`Worktree command failed: git worktree ${args.join(' ')}`, error);
    }
  }

  async list(): Promise<WorktreeData[]> {
    const stdout = await this.execute(['list', '--porcelain']);
    const worktrees: WorktreeData[] = [];
    const lines = stdout.split('\n');
    
    let currentWorktree: Partial<WorktreeData> = {};
    
    for (const line of lines) {
      if (line.startsWith('worktree ')) {
        currentWorktree.path = line.substring(9);
      } else if (line.startsWith('HEAD ')) {
        currentWorktree.head = line.substring(5);
      } else if (line.startsWith('branch ')) {
        currentWorktree.branch = line.substring(7).replace('refs/heads/', '');
      } else if (line === '') {
        if (currentWorktree.path) {
          worktrees.push(currentWorktree as WorktreeData);
          currentWorktree = {};
        }
      }
    }
    
    if (currentWorktree.path) {
      worktrees.push(currentWorktree as WorktreeData);
    }
    
    return worktrees;
  }

  async add(worktreePath: string, branchName: string): Promise<void> {
    await this.execute(['add', worktreePath, branchName]);
  }

  async remove(worktreePath: string, force: boolean = false): Promise<void> {
    const args = ['remove'];
    if (force) args.push('--force');
    args.push(worktreePath);
    await this.execute(args);
  }

  async prune(): Promise<void> {
    await this.execute(['prune']);
  }
}