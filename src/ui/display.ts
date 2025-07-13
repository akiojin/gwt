import chalk from 'chalk';
import { 
  BranchInfo, 
  CleanupTarget 
} from './types.js';
import { WorktreeInfo } from '../worktree.js';
import { getPackageVersion } from '../utils.js';


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

export async function printWelcome(): Promise<void> {
  console.clear();
  console.log();
  
  const version = await getPackageVersion();
  const versionText = version ? ` v${version}` : '';
  const title = `🌳 Claude Worktree Manager${versionText}`;
  
  // Box border configuration
  const borderWidth = 49; // Total width including borders
  const contentWidth = borderWidth - 2; // Width without borders (47)
  
  // Calculate title display width
  // 🌳 = 2 columns, rest is normal text
  const emojiWidth = 2;
  const textLength = title.length - 2; // Subtract emoji bytes
  const titleDisplayWidth = emojiWidth + textLength;
  
  // Calculate right padding
  const rightPadding = contentWidth - titleDisplayWidth - 1; // -1 for left space
  
  console.log(chalk.blueBright.bold('╔═══════════════════════════════════════════════╗'));
  console.log(chalk.blueBright.bold(`║ ${title}${' '.repeat(rightPadding)} ║`));
  console.log(chalk.blueBright.bold('╚═══════════════════════════════════════════════╝'));
  console.log(chalk.gray('Interactive Git worktree manager for Claude Code'));
  console.log(chalk.gray('Press Ctrl+C to quit anytime'));
  console.log();
}

export async function displayBranchTable(): Promise<void> {
  console.clear();
  await printWelcome();
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

export function printExit(): void {
  console.log(chalk.blue('\n👋 Goodbye!'));
}

export async function printStatistics(branches: BranchInfo[], worktrees: WorktreeInfo[]): Promise<void> {
  const localBranches = branches.filter(b => b.type === 'local').length;
  const remoteBranches = branches.filter(b => b.type === 'remote').length;
  const worktreeCount = worktrees.length;
  
  // Count worktrees with changes
  let worktreesWithChanges = 0;
  let totalChangedFiles = 0;
  
  for (const worktree of worktrees) {
    try {
      const { getChangedFilesCount } = await import('../git.js');
      const changedFiles = await getChangedFilesCount(worktree.path);
      if (changedFiles > 0) {
        worktreesWithChanges++;
        totalChangedFiles += changedFiles;
      }
    } catch {
      // Ignore errors
    }
  }
  
  // Create dynamic statistics data
  const stats = [
    { label: 'Local branches', value: localBranches, color: chalk.green.bold },
    { label: 'Remote branches', value: remoteBranches, color: chalk.blue.bold },
    { label: 'Active worktrees', value: worktreeCount, color: chalk.magenta.bold }
  ];
  
  if (worktreesWithChanges > 0) {
    stats.push(
      { label: 'Worktrees with changes', value: worktreesWithChanges, color: chalk.yellow.bold },
      { label: 'Total uncommitted files', value: totalChangedFiles, color: chalk.yellow.bold }
    );
  }
  
  
}

export function displayCleanupTargets(targets: CleanupTarget[]): void {
  console.log(chalk.blue.bold('\n🧹 Merged PR Worktrees:'));
  console.log();
  
  for (const target of targets) {
    const statusIcons = [];
    if (target.hasUncommittedChanges) {
      statusIcons.push(chalk.red('●'));
    }
    if (target.hasUnpushedCommits) {
      statusIcons.push(chalk.yellow('↑'));
    }
    
    const status = statusIcons.length > 0 ? ` ${statusIcons.join(' ')}` : '';
    const prInfo = chalk.gray(`PR #${target.pullRequest.number}: ${target.pullRequest.title}`);
    
    console.log(`  ${chalk.green(target.branch)}${status}`);
    console.log(`    ${prInfo}`);
    console.log(`    ${chalk.gray(target.worktreePath)}`);
    if (target.hasUncommittedChanges) {
      console.log(`    ${chalk.red('⚠️  Has uncommitted changes')}`);
    }
    if (target.hasUnpushedCommits) {
      console.log(`    ${chalk.yellow('⚠️  Has unpushed commits')}`);
    }
    console.log();
  }
}

export function displayCleanupResults(results: Array<{ target: CleanupTarget; success: boolean; error?: string }>): void {
  console.log(chalk.blue.bold('\n🧹 Cleanup Results:'));
  console.log();
  
  let successCount = 0;
  let failureCount = 0;
  
  for (const result of results) {
    if (result.success) {
      successCount++;
      console.log(chalk.green(`  ✅ ${result.target.branch} - Successfully removed`));
    } else {
      failureCount++;
      console.log(chalk.red(`  ❌ ${result.target.branch} - Failed: ${result.error || 'Unknown error'}`));
    }
  }
  
  console.log();
  console.log(chalk.gray(`Summary: ${successCount} succeeded, ${failureCount} failed`));
}