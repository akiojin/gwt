import Table from 'cli-table3';
import chalk from 'chalk';
import stringWidth from 'string-width';
import { BranchInfo } from './types.js';
import { WorktreeInfo } from '../worktree.js';

export interface TableBranchRow {
  branchName: string;
  type: string;
  worktree: string;
  status: string;
  path: string;
  value: string;
}

export function createBranchTable(
  branches: BranchInfo[],
  worktrees: WorktreeInfo[]
): Array<{ name: string; value: string; description?: string }> {
  
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
    
    // Format type with colors
    const typeColor = getBranchTypeColor(branch.branchType);
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
    
    // Create table-like display string - no trailing spaces on last column
    const displayName = [
      padEndUnicode(branchDisplay, 30),
      padEndUnicode(typeText, 10),
      padEndUnicode(worktreeStatus, 8),
      statusText // No padding for the last column
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
    'Status' // Match the header's last column without padding
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
    name: '❌ Exit',
    value: '__exit__',
    description: 'Exit the application'
  });

  return choices;
}

function getBranchTypeColor(branchType: BranchInfo['branchType']) {
  switch (branchType) {
    case 'main':
      return chalk.red.bold;
    case 'develop':
      return chalk.blue.bold;
    case 'feature':
      return chalk.green;
    case 'hotfix':
      return chalk.red;
    case 'release':
      return chalk.yellow;
    default:
      return chalk.gray;
  }
}

function truncatePath(path: string, maxLength: number): string {
  const width = stringWidth(path);
  if (width <= maxLength) return path;
  
  // Show the beginning of the path with ellipsis at the end
  // Account for '...' which takes 3 display width
  const ellipsis = '...';
  const availableWidth = maxLength - stringWidth(ellipsis);
  
  // Start from the beginning
  let result = '';
  let currentWidth = 0;
  
  for (let i = 0; i < path.length; i++) {
    const char = path[i];
    if (!char) continue;
    const charWidth = stringWidth(char);
    
    if (currentWidth + charWidth > availableWidth) {
      break;
    }
    
    result += char;
    currentWidth += charWidth;
  }
  
  return result + ellipsis;
}

function padEndUnicode(str: string, targetLength: number, padString: string = ' '): string {
  const strWidth = stringWidth(str);
  if (strWidth >= targetLength) return str;
  
  const padWidth = targetLength - strWidth;
  return str + padString.repeat(padWidth);
}