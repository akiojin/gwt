import { select, input, confirm, checkbox } from '@inquirer/prompts';
import chalk from 'chalk';
import { 
  BranchInfo, 
  BranchType, 
  NewBranchConfig,
  CleanupTarget
} from './types.js';
import { SessionData } from '../config/index.js';

export async function selectFromTable(
  choices: Array<{ name: string; value: string; description?: string; disabled?: boolean }>,
  statistics?: { branches: BranchInfo[]; worktrees: import('../worktree.js').WorktreeInfo[] }
): Promise<string> {
  
  // Display statistics if provided
  if (statistics) {
    const { printStatistics, printWelcome } = await import('./display.js');
    console.clear();
    await printWelcome();
    await printStatistics(statistics.branches, statistics.worktrees);
  }
  
  return await selectBranchWithShortcuts(choices);
}

async function selectBranchWithShortcuts(
  allChoices: Array<{ name: string; value: string; description?: string; disabled?: boolean }>
): Promise<string> {
  const { createPrompt, useState, useKeypress, isEnterKey, usePrefix } = await import('@inquirer/core');
  
  const branchSelectPrompt = createPrompt<string, { 
    message: string; 
    choices: Array<{ name: string; value: string; description?: string; disabled?: boolean }>;
    pageSize?: number;
  }>((config, done) => {
    const [selectedIndex, setSelectedIndex] = useState(0);
    const [status, setStatus] = useState<'idle' | 'done'>('idle');
    const prefix = usePrefix({});
    
    useKeypress((key) => {
      if (key.name === 'n') {
        setStatus('done');
        done('__create_new__');
        return;
      }
      if (key.name === 'm') {
        setStatus('done');
        done('__manage_worktrees__');
        return;
      }
      if (key.name === 'c') {
        setStatus('done');
        done('__cleanup_prs__');
        return;
      }
      if (key.name === 'q') {
        setStatus('done');
        done('__exit__');
        return;
      }
      
      if (key.name === 'up' || key.name === 'k') {
        setSelectedIndex(Math.max(0, selectedIndex - 1));
        return;
      }
      if (key.name === 'down' || key.name === 'j') {
        // 選択可能な項目数に基づいて制限
        const selectableChoices = config.choices.filter(c => 
          c.value !== '__header__' && 
          c.value !== '__separator__' &&
          !c.disabled
        );
        setSelectedIndex(Math.min(selectableChoices.length - 1, selectedIndex + 1));
        return;
      }
      
      if (isEnterKey(key)) {
        // 選択可能な項目のみから選択
        const selectableChoices = config.choices.filter(c => 
          c.value !== '__header__' && 
          c.value !== '__separator__' &&
          !c.disabled
        );
        const selectedChoice = selectableChoices[selectedIndex];
        if (selectedChoice) {
          setStatus('done');
          done(selectedChoice.value);
        }
        return;
      }
    });
    
    if (status === 'done') {
      return `${prefix} ${config.message}`;
    }
    
    // ヘッダー行とセパレーター行を探す
    const headerChoice = config.choices.find(c => c.value === '__header__');
    const separatorChoice = config.choices.find(c => c.value === '__separator__');
    
    // 選択可能な項目のみをフィルタリング
    const selectableChoices = config.choices.filter(c => 
      c.value !== '__header__' && 
      c.value !== '__separator__' &&
      !c.disabled
    );
    
    const pageSize = config.pageSize || 15;
    
    let output = `${prefix} ${config.message}\n`;
    output += 'Actions: (n) Create new branch, (m) Manage worktrees, (c) Clean up merged PRs, (q) Exit\n\n';
    
    // ヘッダー行とセパレーター行を表示
    if (headerChoice) {
      output += `  ${headerChoice.name}\n`;
    }
    if (separatorChoice) {
      output += `  ${separatorChoice.name}\n`;
    }
    
    // 選択可能な項目のみを表示（ページネーション付き）
    const selectableStartIndex = Math.max(0, selectedIndex - Math.floor(pageSize / 2));
    const selectableEndIndex = Math.min(selectableChoices.length, selectableStartIndex + pageSize);
    const visibleSelectableChoices = selectableChoices.slice(selectableStartIndex, selectableEndIndex);
    
    visibleSelectableChoices.forEach((choice, index) => {
      const globalIndex = selectableStartIndex + index;
      const cursor = globalIndex === selectedIndex ? '❯' : ' ';
      output += `${cursor} ${choice.name}\n`;
    });
    
    return output;
  });
  
  return await branchSelectPrompt({
    message: 'Select a branch:',
    choices: allChoices,
    pageSize: 15
  });
}

export async function selectBranchType(): Promise<BranchType> {
  return await select({
    message: 'Select branch type:',
    choices: [
      {
        name: '🚀 Feature',
        value: 'feature',
        description: 'A new feature branch'
      },
      {
        name: '🔥 Hotfix',
        value: 'hotfix',
        description: 'A critical bug fix'
      },
      {
        name: '📦 Release',
        value: 'release',
        description: 'A release preparation branch'
      }
    ]
  });
}

export async function inputBranchName(type: BranchType): Promise<string> {
  return await input({
    message: `Enter ${type} name:`,
    validate: (value: string) => {
      if (!value.trim()) {
        return 'Branch name cannot be empty';
      }
      if (/[\s\\/:*?"<>|]/.test(value.trim())) {
        return 'Branch name cannot contain spaces or special characters (\\/:*?"<>|)';
      }
      return true;
    },
    transformer: (value: string) => value.trim()
  });
}

export async function selectBaseBranch(branches: BranchInfo[]): Promise<string> {
  const mainBranches = branches.filter(b => 
    b.type === 'local' && (b.branchType === 'main' || b.branchType === 'develop')
  );
  
  if (mainBranches.length === 0) {
    throw new Error('No main or develop branch found');
  }
  
  if (mainBranches.length === 1 && mainBranches[0]) {
    return mainBranches[0].name;
  }
  
  return await select({
    message: 'Select base branch:',
    choices: mainBranches.map(branch => ({
      name: branch.name,
      value: branch.name,
      description: `${branch.branchType} branch`
    }))
  });
}

export async function confirmWorktreeCreation(branchName: string, worktreePath: string): Promise<boolean> {
  return await confirm({
    message: `Create worktree for "${branchName}" at "${worktreePath}"?`,
    default: true
  });
}

export async function confirmWorktreeRemoval(worktreePath: string): Promise<boolean> {
  return await confirm({
    message: `Remove worktree at "${worktreePath}"?`,
    default: false
  });
}

export async function getNewBranchConfig(): Promise<NewBranchConfig> {
  const type = await selectBranchType();
  const taskName = await inputBranchName(type);
  const branchName = `${type}/${taskName}`;
  
  return {
    type,
    taskName,
    branchName
  };
}

export async function confirmSkipPermissions(): Promise<boolean> {
  return await confirm({
    message: 'Skip Claude Code permissions check (--dangerously-skip-permissions)?',
    default: false
  });
}

export async function selectWorktreeForManagement(worktrees: Array<{ branch: string; path: string; isAccessible?: boolean; invalidReason?: string }>): Promise<string | 'back'> {
  const choices = [
    ...worktrees.map((w, index) => {
      const isInvalid = w.isAccessible === false;
      return {
        name: isInvalid 
          ? chalk.red(`${index + 1}. ✗ ${w.branch}`)
          : `${index + 1}. ${w.branch}`,
        value: w.branch,
        description: isInvalid 
          ? chalk.red(`${w.path} (${w.invalidReason || 'Inaccessible'})`)
          : w.path,
        disabled: isInvalid ? 'Cannot access this worktree' : false
      };
    }),
    {
      name: '← Back to main menu',
      value: 'back',
      description: 'Return to main menu'
    }
  ];

  return await select({
    message: 'Select worktree to manage:',
    choices,
    pageSize: 15
  });
}

export async function selectWorktreeAction(): Promise<'open' | 'remove' | 'remove-branch' | 'back'> {
  return await select({
    message: 'What would you like to do?',
    choices: [
      {
        name: '📂 Open in Claude Code',
        value: 'open',
        description: 'Launch Claude Code in this worktree'
      },
      {
        name: '🗑️  Remove worktree',
        value: 'remove',
        description: 'Delete this worktree only'
      },
      {
        name: '🔥 Remove worktree and branch',
        value: 'remove-branch',
        description: 'Delete both worktree and branch'
      },
      {
        name: '← Back',
        value: 'back',
        description: 'Return to worktree list'
      }
    ]
  });
}

export async function confirmBranchRemoval(branchName: string): Promise<boolean> {
  return await confirm({
    message: `Are you sure you want to delete the branch "${branchName}"? This cannot be undone.`,
    default: false
  });
}

export async function selectChangesAction(): Promise<'status' | 'commit' | 'stash' | 'discard' | 'continue'> {
  return await select({
    message: 'Changes detected in worktree. What would you like to do?',
    choices: [
      {
        name: '📋 View changes (git status)',
        value: 'status',
        description: 'Show modified files'
      },
      {
        name: '💾 Commit changes',
        value: 'commit',
        description: 'Create a new commit'
      },
      {
        name: '📦 Stash changes',
        value: 'stash',
        description: 'Save changes for later'
      },
      {
        name: '🗑️  Discard changes',
        value: 'discard',
        description: 'Discard all changes (careful!)'
      },
      {
        name: '➡️  Continue without action',
        value: 'continue',
        description: 'Return to main menu'
      }
    ]
  });
}

export async function inputCommitMessage(): Promise<string> {
  return await input({
    message: 'Enter commit message:',
    validate: (value: string) => {
      if (!value.trim()) {
        return 'Commit message cannot be empty';
      }
      return true;
    }
  });
}

export async function confirmDiscardChanges(): Promise<boolean> {
  return await confirm({
    message: 'Are you sure you want to discard all changes? This cannot be undone.',
    default: false
  });
}

export async function confirmContinue(message = 'Continue?'): Promise<boolean> {
  return await confirm({
    message,
    default: true
  });
}

export async function selectCleanupTargets(targets: CleanupTarget[]): Promise<CleanupTarget[]> {
  if (targets.length === 0) {
    return [];
  }
  
  const choices = targets.map(target => ({
    name: `${target.branch} (PR #${target.pullRequest.number}: ${target.pullRequest.title})`,
    value: target,
    disabled: target.hasUncommittedChanges 
      ? 'Has uncommitted changes' 
      : false,
    checked: !target.hasUncommittedChanges
  }));
  
  const selected = await checkbox({
    message: 'Select worktrees to clean up (merged PRs):',
    choices,
    pageSize: 15,
    instructions: 'Space to select, Enter to confirm'
  });
  
  return selected;
}

export async function confirmCleanup(targets: CleanupTarget[]): Promise<boolean> {
  const message = targets.length === 1 && targets[0]
    ? `Delete worktree and branch "${targets[0].branch}"?`
    : `Delete ${targets.length} worktrees and their branches?`;
    
  return await confirm({
    message,
    default: false
  });
}

export async function confirmRemoteBranchDeletion(targets: CleanupTarget[]): Promise<boolean> {
  const message = targets.length === 1 && targets[0]
    ? `Also delete remote branch "${targets[0].branch}"?`
    : `Also delete ${targets.length} remote branches?`;
    
  return await confirm({
    message,
    default: false
  });
}

export async function confirmPushUnpushedCommits(targets: CleanupTarget[]): Promise<boolean> {
  const branchesWithUnpushed = targets.filter(t => t.hasUnpushedCommits);
  
  if (branchesWithUnpushed.length === 0) {
    return false;
  }
  
  const message = branchesWithUnpushed.length === 1 && branchesWithUnpushed[0]
    ? `Push unpushed commits in "${branchesWithUnpushed[0].branch}" before deletion?`
    : `Push unpushed commits in ${branchesWithUnpushed.length} branches before deletion?`;
    
  return await confirm({
    message,
    default: true
  });
}

export async function confirmProceedWithoutPush(branchName: string): Promise<boolean> {
  return await confirm({
    message: `Failed to push "${branchName}". Proceed with deletion anyway?`,
    default: false
  });
}

export async function selectSession(sessions: SessionData[]): Promise<SessionData | null> {
  if (sessions.length === 0) {
    return null;
  }

  console.log('\n┌─────────────────────────────────────────────────────────────────────────────────┐');
  console.log('│                            📋 Resume Claude Code Session                        │');
  console.log('└─────────────────────────────────────────────────────────────────────────────────┘\n');

  // Collect enhanced session information
  const enhancedChoices = await Promise.all(
    sessions.map(async (session, index) => {
      if (!session.lastWorktreePath || !session.lastBranch) {
        // Fallback to simple display for incomplete sessions
        const repo = session.repositoryRoot.split('/').pop() || 'unknown';
        const timeAgo = formatTimeAgo(session.timestamp);
        const branch = session.lastBranch || 'unknown';
        
        return {
          name: `${chalk.cyan(repo)} - ${chalk.green(branch)} ${chalk.gray(`(${timeAgo})`)}`,
          value: index.toString(),
          description: session.lastWorktreePath || ''
        };
      }

      try {
        const { getEnhancedSessionInfo } = await import('../git.js');
        const sessionInfo = await getEnhancedSessionInfo(session.lastWorktreePath, session.lastBranch);
        return formatEnhancedSessionDisplay(session, sessionInfo, index);
      } catch {
        // Fallback to simple display if enhanced info fails
        const repo = session.repositoryRoot.split('/').pop() || 'unknown';
        const timeAgo = formatTimeAgo(session.timestamp);
        const branch = session.lastBranch || 'unknown';
        
        return {
          name: `${chalk.cyan(repo)} - ${chalk.green(branch)} ${chalk.gray(`(${timeAgo})`)} ${chalk.red('(info unavailable)')}`,
          value: index.toString(),
          description: session.lastWorktreePath || ''
        };
      }
    })
  );

  const selectedIndex = await select({
    message: 'Select a session to resume:',
    choices: [
      ...enhancedChoices,
      { name: chalk.gray('← Cancel'), value: 'cancel' }
    ],
    pageSize: 8
  });

  if (selectedIndex === 'cancel') {
    return null;
  }

  return sessions[parseInt(selectedIndex)] || null;
}

export async function selectClaudeExecutionMode(): Promise<{
  mode: 'normal' | 'continue' | 'resume';
  skipPermissions: boolean;
}> {
  const mode = await select({
    message: 'Select Claude Code execution mode:',
    choices: [
      {
        name: '🚀 Normal - Start a new session',
        value: 'normal',
        description: 'Launch Claude Code normally'
      },
      {
        name: '⏭️  Continue - Continue most recent conversation (-c)',
        value: 'continue',
        description: 'Continue from the most recent conversation'
      },
      {
        name: '🔄 Resume - Select conversation to resume (-r)',
        value: 'resume',
        description: 'Interactively select a conversation to resume'
      }
    ],
    pageSize: 3
  }) as 'normal' | 'continue' | 'resume';

  const skipPermissions = await confirm({
    message: 'Skip permission checks? (--dangerously-skip-permissions)',
    default: false
  });

  return { mode, skipPermissions };
}

function formatTimeAgo(timestamp: number): string {
  const now = Date.now();
  const diff = now - timestamp;
  
  const minutes = Math.floor(diff / (1000 * 60));
  const hours = Math.floor(diff / (1000 * 60 * 60));
  const days = Math.floor(diff / (1000 * 60 * 60 * 24));
  
  if (minutes < 60) {
    return `${minutes}m ago`;
  } else if (hours < 24) {
    return `${hours}h ago`;
  } else {
    return `${days}d ago`;
  }
}

/**
 * Get branch icon based on branch type
 */
function getBranchIcon(branchType: string): string {
  switch (branchType) {
    case 'feature': return '🌿';
    case 'bugfix': return '🐛';
    case 'hotfix': return '🔥';
    case 'develop': return '🌱';
    case 'main':
    case 'master': return '🎯';
    default: return '📝';
  }
}

/**
 * Get project icon based on repository name
 */
function getProjectIcon(repoName: string): string {
  const lowerName = repoName.toLowerCase();
  
  if (lowerName.includes('app') || lowerName.includes('mobile')) return '📱';
  if (lowerName.includes('api') || lowerName.includes('backend')) return '⚡';
  if (lowerName.includes('frontend') || lowerName.includes('ui')) return '🎨';
  if (lowerName.includes('cli') || lowerName.includes('tool')) return '🛠️';
  if (lowerName.includes('bot') || lowerName.includes('ai')) return '🤖';
  if (lowerName.includes('web')) return '🌐';
  if (lowerName.includes('doc') || lowerName.includes('guide')) return '📚';
  
  return '🚀';
}

/**
 * Get status display with icon and color
 */
function getStatusDisplay(sessionInfo: import('../git.js').EnhancedSessionInfo): { text: string; color: string } {
  if (sessionInfo.hasUncommittedChanges) {
    const count = sessionInfo.uncommittedChangesCount;
    return {
      text: `📝 ${count} uncommitted change${count !== 1 ? 's' : ''}`,
      color: 'yellow'
    };
  }
  
  if (sessionInfo.hasUnpushedCommits) {
    const count = sessionInfo.unpushedCommitsCount;
    return {
      text: `⚠️  Needs push (${count} commit${count !== 1 ? 's' : ''})`,
      color: 'yellow'
    };
  }
  
  return {
    text: '✅ All changes committed',
    color: 'green'
  };
}

/**
 * Format enhanced session display with better visual layout
 */
function formatEnhancedSessionDisplay(
  session: SessionData, 
  sessionInfo: import('../git.js').EnhancedSessionInfo,
  index: number
): { name: string; value: string; description?: string } {
  const repo = session.repositoryRoot.split('/').pop() || 'unknown';
  const timeAgo = formatTimeAgo(session.timestamp);
  const branch = session.lastBranch || 'unknown';
  
  const projectIcon = getProjectIcon(repo);
  const branchIcon = getBranchIcon(sessionInfo.branchType);
  const statusDisplay = getStatusDisplay(sessionInfo);
  
  // Create commit message display (truncate if too long)
  const commitMsg = sessionInfo.latestCommitMessage || 'No commit message';
  const truncatedCommit = commitMsg.length > 50 ? commitMsg.substring(0, 47) + '...' : commitMsg;
  
  // Format status text with proper colors
  let coloredStatusText: string;
  if (statusDisplay.color === 'yellow') {
    coloredStatusText = chalk.yellow(statusDisplay.text);
  } else if (statusDisplay.color === 'green') {
    coloredStatusText = chalk.green(statusDisplay.text);
  } else {
    coloredStatusText = chalk.red(statusDisplay.text);
  }
  
  // Format the display with box-like structure
  const topLine = `┌─ ${projectIcon} ${chalk.bold(repo)} ${' '.repeat(Math.max(0, 60 - repo.length))} ${chalk.gray(timeAgo)} ─┐`;
  const branchLine = `│ ${branchIcon} ${chalk.green(branch)} ${' '.repeat(Math.max(0, 35 - branch.length))} ${coloredStatusText} ${' '.repeat(Math.max(0, 15))} │`;
  const commitLine = `│ 💬 ${chalk.gray(`"${truncatedCommit}"`)} ${' '.repeat(Math.max(0, 65 - truncatedCommit.length))} │`;
  const pathLine = `│ 📁 ${chalk.dim(session.lastWorktreePath || '')} ${' '.repeat(Math.max(0, 65 - (session.lastWorktreePath || '').length))} │`;
  const bottomLine = `└${'─'.repeat(77)}┘`;
  
  const fullDisplay = [topLine, branchLine, commitLine, pathLine, bottomLine].join('\n');
  
  return {
    name: fullDisplay,
    value: index.toString(),
    description: ''
  };
}


