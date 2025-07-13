import chalk from 'chalk';
import { BranchInfo, BranchChoice } from './types.js';
import { WorktreeInfo } from '../worktree.js';

export function formatBranchForDisplay(branch: BranchInfo, worktreeInfo?: WorktreeInfo): BranchChoice {
  const { name, type, branchType, isCurrent } = branch;
  
  let displayName = name;
  let description = '';
  
  // Add type and current indicators
  const indicators: string[] = [];
  
  if (isCurrent) {
    indicators.push(chalk.green('current'));
  }
  
  if (type === 'remote') {
    indicators.push(chalk.cyan('remote'));
  } else {
    indicators.push(chalk.yellow('local'));
  }
  
  // Add branch type color
  const typeColor = getBranchTypeColor(branchType);
  indicators.push(typeColor(branchType));
  
  // Add worktree status
  if (worktreeInfo) {
    indicators.push(chalk.magenta('worktree'));
    description = `Has worktree at: ${worktreeInfo.path}`;
  }
  
  displayName = `${name} ${chalk.gray(`(${indicators.join(', ')})`)}`;
  
  return {
    name: displayName,
    value: name,
    description
  };
}

export function getBranchTypeColor(branchType: BranchInfo['branchType']) {
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

export function printWelcome(): void {
  console.log();
  console.log(chalk.blue.bold('🌳 Claude Worktree Manager'));
  console.log(chalk.gray('Interactive Git worktree manager for Claude Code'));
  console.log();
}

export function printSuccess(message: string): void {
  console.log(chalk.green('✅'), message);
}

export function printError(message: string): void {
  console.log(chalk.red('❌'), message);
}

export function printInfo(message: string): void {
  console.log(chalk.blue('ℹ️'), message);
}

export function printWarning(message: string): void {
  console.log(chalk.yellow('⚠️'), message);
}

export function formatWorktreesList(worktrees: WorktreeInfo[]): void {
  if (worktrees.length === 0) {
    console.log(chalk.gray('No worktrees found.'));
    return;
  }
  
  console.log(chalk.blue.bold('📁 Existing Worktrees:'));
  console.log();
  
  for (const worktree of worktrees) {
    console.log(`  ${chalk.cyan(worktree.branch)} → ${chalk.gray(worktree.path)}`);
  }
  
  console.log();
}