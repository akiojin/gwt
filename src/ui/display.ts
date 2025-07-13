import chalk from 'chalk';
import stringWidth from 'string-width';
import { 
  BranchInfo, 
  EnhancedBranchChoice, 
  BranchGroup,
  UIFilter 
} from './types.js';
import { WorktreeInfo } from '../worktree.js';

export function createEnhancedBranchChoice(
  branch: BranchInfo, 
  worktreeInfo?: WorktreeInfo
): EnhancedBranchChoice {
  const { name, type, branchType, isCurrent } = branch;
  
  let displayName = name;
  let description = '';
  
  // Build status indicators
  const indicators: string[] = [];
  
  if (isCurrent) {
    indicators.push(chalk.green('★ current'));
  }
  
  if (type === 'remote') {
    indicators.push(chalk.cyan('remote'));
  } else {
    indicators.push(chalk.yellow('local'));
  }
  
  // Add branch type with color
  const typeColor = getBranchTypeColor(branchType);
  indicators.push(typeColor(branchType));
  
  // Add worktree status
  const hasWorktree = !!worktreeInfo;
  if (hasWorktree) {
    indicators.push(chalk.magenta('📁 worktree'));
    description = `Worktree: ${worktreeInfo.path}`;
  } else {
    indicators.push(chalk.gray('no worktree'));
  }
  
  displayName = `${name} ${chalk.gray(`(${indicators.join(' • ')})`)}`;
  
  const result: EnhancedBranchChoice = {
    name: displayName,
    value: name,
    description,
    hasWorktree,
    branchType,
    branchDataType: type,
    isCurrent
  };

  if (worktreeInfo?.path) {
    result.worktreePath = worktreeInfo.path;
  }

  return result;
}

export function createBranchGroups(
  branches: BranchInfo[],
  worktrees: WorktreeInfo[],
  filter?: UIFilter
): BranchGroup[] {
  // Create worktree lookup map
  const worktreeMap = new Map(worktrees.map(w => [w.branch, w]));
  
  // Convert to enhanced choices
  const enhancedBranches = branches.map(branch => 
    createEnhancedBranchChoice(branch, worktreeMap.get(branch.name))
  );
  
  // Apply filters if provided
  const filteredBranches = filter ? applyFilter(enhancedBranches, filter) : enhancedBranches;
  
  // Group branches
  const groups: BranchGroup[] = [
    {
      title: '🔥 Current & Main Branches',
      branches: filteredBranches.filter(b => 
        b.isCurrent || b.branchType === 'main' || b.branchType === 'develop'
      ),
      priority: 1
    },
    {
      title: '📁 Branches with Worktrees',
      branches: filteredBranches.filter(b => 
        b.hasWorktree && !b.isCurrent && b.branchType !== 'main' && b.branchType !== 'develop'
      ),
      priority: 2
    },
    {
      title: '🚀 Feature Branches',
      branches: filteredBranches.filter(b => 
        b.branchType === 'feature' && !b.hasWorktree && !b.isCurrent
      ),
      priority: 3
    },
    {
      title: '🔧 Hotfix & Release Branches',
      branches: filteredBranches.filter(b => 
        (b.branchType === 'hotfix' || b.branchType === 'release') && 
        !b.hasWorktree && !b.isCurrent
      ),
      priority: 4
    },
    {
      title: '📥 Remote Branches',
      branches: filteredBranches.filter(b => 
        b.branchDataType === 'remote' && !b.isCurrent
      ),
      priority: 5
    },
    {
      title: '📂 Other Branches',
      branches: filteredBranches.filter(b => 
        b.branchType === 'other' && 
        b.branchDataType === 'local' && 
        !b.hasWorktree && 
        !b.isCurrent
      ),
      priority: 6
    }
  ];
  
  // Remove empty groups
  return groups.filter(group => group.branches.length > 0);
}

export function applyFilter(branches: EnhancedBranchChoice[], filter: UIFilter): EnhancedBranchChoice[] {
  return branches.filter(branch => {
    // Worktree filter
    if (!filter.showWithWorktree && branch.hasWorktree) return false;
    if (!filter.showWithoutWorktree && !branch.hasWorktree) return false;
    
    // Branch type filter
    if (!filter.branchTypes.includes(branch.branchType)) return false;
    
    // Local/Remote filter
    if (!filter.showLocal && branch.branchDataType === 'local') return false;
    if (!filter.showRemote && branch.branchDataType === 'remote') return false;
    
    return true;
  });
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
  console.clear();
  console.log();
  console.log(chalk.blue.bold('🌳 Claude Worktree Manager'));
  console.log(chalk.gray('Interactive Git worktree manager for Claude Code'));
  console.log(chalk.gray('Press Ctrl+C to quit anytime'));
  console.log();
}

export function displayBranchTable(): void {
  console.clear();
  printWelcome();
  
  // Display table header - add 2 spaces for cursor alignment
  const headerParts = [
    padEndUnicode('Branch Name', 30),
    padEndUnicode('Type', 10),
    padEndUnicode('Worktree', 8),
    'Status' // No padding for the last column
  ];
  const header = '  ' + headerParts.join(' │ '); // 2 spaces for cursor
  console.log(chalk.blue.bold(header));
  console.log('  ' + '─'.repeat(stringWidth(header) - 2)); // 2 spaces + adjusted line
}

function padEndUnicode(str: string, targetLength: number, padString: string = ' '): string {
  const strWidth = stringWidth(str);
  if (strWidth >= targetLength) return str;
  
  const padWidth = targetLength - strWidth;
  return str + padString.repeat(padWidth);
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

export function formatWorktreesList(worktrees: WorktreeInfo[]): void {
  if (worktrees.length === 0) {
    console.log(chalk.gray('No worktrees found.'));
    return;
  }
  
  console.log(chalk.blue.bold('\n📁 Existing Worktrees:'));
  console.log();
  
  for (const worktree of worktrees) {
    const branchColor = getBranchTypeColor(
      worktree.branch.startsWith('feature/') ? 'feature' :
      worktree.branch.startsWith('hotfix/') ? 'hotfix' :
      worktree.branch.startsWith('release/') ? 'release' :
      worktree.branch === 'main' ? 'main' :
      worktree.branch === 'develop' ? 'develop' : 'other'
    );
    
    console.log(`  ${branchColor(worktree.branch)} → ${chalk.gray(worktree.path)}`);
  }
  
  console.log();
}

export function printStatistics(branches: BranchInfo[], worktrees: WorktreeInfo[]): void {
  const localBranches = branches.filter(b => b.type === 'local').length;
  const remoteBranches = branches.filter(b => b.type === 'remote').length;
  const worktreeCount = worktrees.length;
  
  console.log(chalk.gray('📊 Repository Statistics:'));
  console.log(chalk.gray(`   Local branches: ${localBranches}`));
  console.log(chalk.gray(`   Remote branches: ${remoteBranches}`));
  console.log(chalk.gray(`   Active worktrees: ${worktreeCount}`));
  console.log();
}