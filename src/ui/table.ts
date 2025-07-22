import chalk from 'chalk';
import stringWidth from 'string-width';
import { BranchInfo } from './types.js';
import { WorktreeInfo } from '../worktree.js';
import { getChangedFilesCount } from '../git.js';


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

  const choices: Array<{ name: string; value: string; description?: string; disabled?: boolean }> = [];

  // Set fixed width for branch name column
  const branchNameColumnWidth = 32;

  // Add header row
  const headerRow = [
    padEndUnicode(chalk.bold.cyan('ブランチ名'), branchNameColumnWidth),
    padEndUnicode(chalk.bold.cyan('タイプ'), 14),
    padEndUnicode(chalk.bold.cyan('Worktree'), 10),
    padEndUnicode(chalk.bold.cyan('ステータス'), 12),
    chalk.bold.cyan('変更')
  ].join(' ┃ ');

  choices.push({
    name: headerRow,
    value: '__header__',
    disabled: true
  });

  // Add separator row
  const separatorRow = [
    '─'.repeat(branchNameColumnWidth),
    '─'.repeat(14),
    '─'.repeat(10),
    '─'.repeat(12),
    '─'.repeat(4)
  ].join('─┼─');

  choices.push({
    name: chalk.gray(separatorRow),
    value: '__separator__',
    disabled: true
  });

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
      // Check if worktree is accessible
      if (worktree.isAccessible === false) {
        changesText = chalk.red('✗ Invalid');
      } else {
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
      }
    } else {
      changesText = chalk.gray('─');
    }
    
    // Create table-like display string with truncated branch names
    const displayName = [
      padEndUnicode(truncateString(branchDisplay, branchNameColumnWidth), branchNameColumnWidth),
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

function padEndUnicode(str: string, targetLength: number, padString = ' '): string {
  const strWidth = stringWidth(str);
  if (strWidth >= targetLength) return str;
  
  const padWidth = targetLength - strWidth;
  return str + padString.repeat(Math.max(0, padWidth));
}

function truncateString(str: string, maxWidth: number): string {
  const strWidth = stringWidth(str);
  if (strWidth <= maxWidth) return str;
  
  // Try to truncate while preserving meaning
  let truncated = str;
  const ellipsis = '...';
  const ellipsisWidth = stringWidth(ellipsis);
  const targetWidth = maxWidth - ellipsisWidth;
  
  while (stringWidth(truncated) > targetWidth && truncated.length > 0) {
    truncated = truncated.slice(0, -1);
  }
  
  return truncated + ellipsis;
}