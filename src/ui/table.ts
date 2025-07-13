import stringWidth from 'string-width';
import { BranchInfo } from './types.js';
import { WorktreeInfo } from '../worktree.js';
import { getChangedFilesCount } from '../git.js';

export interface TableBranchRow {
  branchName: string;
  type: string;
  worktree: string;
  status: string;
  path: string;
  value: string;
}

export async function createBranchTable(
  branches: BranchInfo[],
  worktrees: WorktreeInfo[]
): Promise<Array<{ name: string; value: string; description?: string }>> {
  
  // Create worktree lookup map (excluding main repository)
  const worktreeMap = new Map(
    worktrees
      .filter(w => w.path !== process.cwd()) // Exclude main repository
      .map(w => [w.branch, w])
  );

  const choices: Array<{ name: string; value: string; description?: string }> = [];

  // Filter out "origin" branch and sort: current first, then by type
  const filteredBranches = branches.filter(b => b.name !== 'origin');
  const sortedBranches = [...filteredBranches].sort((a, b) => {
    if (a.isCurrent && !b.isCurrent) return -1;
    if (!a.isCurrent && b.isCurrent) return 1;
    if (a.branchType === 'main' && b.branchType !== 'main') return -1;
    if (a.branchType !== 'main' && b.branchType === 'main') return 1;
    return a.name.localeCompare(b.name);
  });

  for (const branch of sortedBranches) {
    const worktree = worktreeMap.get(branch.name);
    const hasWorktree = !!worktree;
    
    // Format branch name with indicators
    let branchDisplay = branch.name;
    if (branch.isCurrent) {
      branchDisplay = `★ ${branch.name}`;
    }
    
    // Format type
    const typeText = branch.branchType;
    
    // Format worktree status
    const worktreeStatus = hasWorktree ? '✓' : '';
    
    // Format status
    let statusText = '';
    if (branch.isCurrent) {
      statusText = 'current';
    } else if (branch.type === 'remote') {
      statusText = 'remote';
    } else {
      statusText = 'local';
    }
    
    // Get changes count if worktree exists
    let changesText = '';
    if (hasWorktree && worktree) {
      try {
        const changedFiles = await getChangedFilesCount(worktree.path);
        if (changedFiles > 0) {
          changesText = `${changedFiles} files`;
        }
      } catch {
        // Ignore errors when getting change count
      }
    }
    
    // Create table-like display string - no trailing spaces on last column
    const displayName = [
      padEndUnicode(branchDisplay, 30),
      padEndUnicode(typeText, 10),
      padEndUnicode(worktreeStatus, 8),
      padEndUnicode(statusText, 10),
      changesText // No padding for the last column
    ].join(' │ ');

    choices.push({
      name: displayName,
      value: branch.name,
      description: hasWorktree ? `Worktree: ${worktree.path}` : 'No worktree'
    });
  }

  // Add separator - match the exact width of table rows
  // Create a sample row to measure its width (matching the header format)
  const sampleRow = [
    padEndUnicode('', 30),
    padEndUnicode('', 10),
    padEndUnicode('', 8),
    padEndUnicode('', 10),
    'Changes' // Match the header's last column without padding
  ].join(' │ ');
  
  choices.push({
    name: '─'.repeat(stringWidth(sampleRow)),
    value: '__separator__',
    description: ''
  });

  // Add special choices
  choices.push({
    name: '✨ Create new branch',
    value: '__create_new__',
    description: 'Create a new feature, hotfix, or release branch'
  });

  choices.push({
    name: '🗂️  Manage worktrees',
    value: '__manage_worktrees__',
    description: 'View and manage existing worktrees'
  });

  choices.push({
    name: '🧹 Clean up merged PRs',
    value: '__cleanup_prs__',
    description: 'Remove worktrees and branches for merged pull requests'
  });

  choices.push({
    name: '❌ Exit',
    value: '__exit__',
    description: 'Exit the application'
  });

  return choices;
}

function padEndUnicode(str: string, targetLength: number, padString: string = ' '): string {
  const strWidth = stringWidth(str);
  if (strWidth >= targetLength) return str;
  
  const padWidth = targetLength - strWidth;
  return str + padString.repeat(padWidth);
}