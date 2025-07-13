import chalk from 'chalk';
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
      branchDisplay = `◉ ${branch.name}`;
    }
    
    // Format type with colors and icons
    const typeIcon = getBranchTypeIcon(branch.branchType);
    const typeText = `${typeIcon} ${branch.branchType}`;
    
    // Format worktree status
    const worktreeStatus = hasWorktree ? chalk.green('●') : chalk.gray('○');
    
    // Format status with colors
    let statusText = '';
    if (branch.isCurrent) {
      statusText = chalk.bgGreen.black(' CURRENT ');
    } else if (branch.type === 'remote') {
      statusText = chalk.bgBlue.white(' REMOTE ');
    } else {
      statusText = chalk.bgGray.white(' LOCAL ');
    }
    
    // Get changes count if worktree exists
    let changesText = '';
    if (hasWorktree && worktree) {
      try {
        const changedFiles = await getChangedFilesCount(worktree.path);
        if (changedFiles > 0) {
          changesText = chalk.yellow(`✎ ${changedFiles}`);
        } else {
          changesText = chalk.gray('─');
        }
      } catch {
        changesText = chalk.gray('─');
      }
    } else {
      changesText = chalk.gray('─');
    }
    
    // Create table-like display string with modern separators
    const displayName = [
      padEndUnicode(branchDisplay, 32),
      padEndUnicode(typeText, 14),
      padEndUnicode(worktreeStatus, 10),
      padEndUnicode(statusText, 12),
      changesText // No padding for the last column
    ].join(' ┃ ');

    choices.push({
      name: displayName,
      value: branch.name,
      description: hasWorktree ? `Worktree: ${worktree.path}` : 'No worktree'
    });
  }

  // Add separator with modern box drawing characters
  const sampleRow = [
    padEndUnicode('', 32),
    padEndUnicode('', 14),
    padEndUnicode('', 10),
    padEndUnicode('', 12),
    'Changes'
  ].join(' ┃ ');
  
  choices.push({
    name: '━'.repeat(stringWidth(sampleRow)),
    value: '__separator__',
    description: ''
  });

  // Add special choices with improved formatting
  choices.push({
    name: chalk.cyan('➕ Create new branch'),
    value: '__create_new__',
    description: 'Create a new feature, hotfix, or release branch'
  });

  choices.push({
    name: chalk.magenta('📂 Manage worktrees'),
    value: '__manage_worktrees__',
    description: 'View and manage existing worktrees'
  });

  choices.push({
    name: chalk.red('◈ Exit'),
    value: '__exit__',
    description: 'Exit the application'
  });

  return choices;
}


function getBranchTypeIcon(branchType: BranchInfo['branchType']): string {
  switch (branchType) {
    case 'main':
      return '⚡';
    case 'develop':
      return '🔧';
    case 'feature':
      return '✨';
    case 'hotfix':
      return '🔥';
    case 'release':
      return '🚀';
    default:
      return '📌';
  }
}


function padEndUnicode(str: string, targetLength: number, padString: string = ' '): string {
  const strWidth = stringWidth(str);
  if (strWidth >= targetLength) return str;
  
  const padWidth = targetLength - strWidth;
  return str + padString.repeat(padWidth);
}