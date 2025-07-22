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
  
  // Calculate title display width
  // 🌳 = 2 columns, rest is normal text
  const emojiWidth = 2;
  const textAfterEmoji = title.substring(2).length; // Get length of text after emoji
  const titleDisplayWidth = emojiWidth + textAfterEmoji;
  
  // Determine box width dynamically based on title length
  // Minimum width of 49, or title width + padding
  const minBorderWidth = 49;
  const neededWidth = titleDisplayWidth + 4; // +4 for "║ " and " ║"
  const borderWidth = Math.max(minBorderWidth, neededWidth);
  const contentWidth = borderWidth - 2; // Width without borders
  
  // Calculate right padding
  const rightPadding = contentWidth - titleDisplayWidth - 1; // -1 for left space
  
  // Create border strings
  const topBorder = '╔' + '═'.repeat(borderWidth - 2) + '╗';
  const bottomBorder = '╚' + '═'.repeat(borderWidth - 2) + '╝';
  
  console.log(chalk.blueBright.bold(topBorder));
  console.log(chalk.blueBright.bold(`║ ${title}${' '.repeat(rightPadding)} ║`));
  console.log(chalk.blueBright.bold(bottomBorder));
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
  const invalidWorktrees = worktrees.filter(w => w.isAccessible === false).length;
  
  // Count worktrees with changes
  let worktreesWithChanges = 0;
  let totalChangedFiles = 0;
  
  for (const worktree of worktrees) {
    // Skip inaccessible worktrees
    if (worktree.isAccessible === false) {
      continue;
    }
    
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
  
  if (invalidWorktrees > 0) {
    stats.push({ label: 'Invalid worktrees', value: invalidWorktrees, color: chalk.red.bold });
  }
  
  if (worktreesWithChanges > 0) {
    stats.push(
      { label: 'Worktrees with changes', value: worktreesWithChanges, color: chalk.yellow.bold },
      { label: 'Total uncommitted files', value: totalChangedFiles, color: chalk.yellow.bold }
    );
  }
  
  // Display statistics
  console.log();
  stats.forEach(stat => {
    console.log(chalk.gray('  ') + stat.color(stat.value.toString().padStart(3)) + chalk.gray(' ') + stat.label);
  });
  console.log();
}

export function displayCleanupTargets(targets: CleanupTarget[]): void {
  const worktreeTargets = targets.filter(t => t.cleanupType === 'worktree-and-branch');
  const branchOnlyTargets = targets.filter(t => t.cleanupType === 'branch-only');
  
  if (worktreeTargets.length > 0) {
    console.log(chalk.blue.bold('\n🧹 Merged PR Worktrees (Worktree + Local Branch):'));
    console.log();
    
    for (const target of worktreeTargets) {
      const statusIcons = [];
      if (target.hasUncommittedChanges) {
        statusIcons.push(chalk.red('●'));
      }
      if (target.hasUnpushedCommits) {
        statusIcons.push(chalk.yellow('↑'));
      }
      if (target.hasRemoteBranch) {
        statusIcons.push(chalk.blue('🌐'));
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
        console.log(`    ${chalk.yellow('⚠️  Has unpushed commits (will be pushed before deletion)')}`);
      }
      if (target.hasRemoteBranch) {
        console.log(`    ${chalk.blue('ℹ️  Has remote branch (will be deleted if selected)')}`);
      }
      console.log();
    }
  }
  
  if (branchOnlyTargets.length > 0) {
    console.log(chalk.cyan.bold('\n🌿 Merged PR Local Branches (Local Branch Only):'));
    console.log();
    
    for (const target of branchOnlyTargets) {
      const statusIcons = [];
      if (target.hasRemoteBranch) {
        statusIcons.push(chalk.blue('🌐'));
      } else {
        statusIcons.push(chalk.gray('📍'));
      }
      
      const status = statusIcons.length > 0 ? ` ${statusIcons.join(' ')}` : '';
      const prInfo = chalk.gray(`PR #${target.pullRequest.number}: ${target.pullRequest.title}`);
      
      console.log(`  ${chalk.cyan(target.branch)}${status}`);
      console.log(`    ${prInfo}`);
      if (target.hasRemoteBranch) {
        console.log(`    ${chalk.blue('ℹ️  Has remote branch (will be deleted if selected)')}`);
      } else {
        console.log(`    ${chalk.gray('ℹ️  Local branch only (no remote)')}`);
      }
      console.log();
    }
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