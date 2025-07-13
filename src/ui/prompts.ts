import { select, input, confirm, checkbox } from '@inquirer/prompts';
import { 
  BranchInfo, 
  BranchType, 
  NewBranchConfig,
  CleanupTarget
} from './types.js';

export async function selectFromTable(
  choices: Array<{ name: string; value: string; description?: string; disabled?: boolean }>,
  statistics?: { branches: BranchInfo[]; worktrees: import('../worktree.js').WorktreeInfo[] }
): Promise<string> {
  // Filter branch choices only (exclude action items and header/separator rows)
  const branchChoices = choices.filter(choice => 
    choice.value !== '__separator__' && 
    choice.value !== '__separator_space__' && 
    choice.value !== '__separator_space2__' && 
    choice.value !== '__actions_header__' &&
    choice.value !== '__create_new__' &&
    choice.value !== '__manage_worktrees__' &&
    choice.value !== '__cleanup_prs__' &&
    choice.value !== '__exit__' &&
    choice.value !== '__header__' &&
    choice.name.trim() !== '' &&
    !choice.disabled
  );
  
  // Display statistics if provided
  if (statistics) {
    const { printStatistics } = await import('./display.js');
    await printStatistics(statistics.branches, statistics.worktrees);
  }
  
  return await selectBranchWithShortcuts(branchChoices);
}

async function selectBranchWithShortcuts(
  branchChoices: Array<{ name: string; value: string; description?: string; disabled?: boolean }>
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
        setSelectedIndex(Math.min(config.choices.length - 1, selectedIndex + 1));
        return;
      }
      
      if (isEnterKey(key)) {
        const selectedChoice = config.choices[selectedIndex];
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
    
    const pageSize = config.pageSize || 15;
    const startIndex = Math.max(0, selectedIndex - Math.floor(pageSize / 2));
    const endIndex = Math.min(config.choices.length, startIndex + pageSize);
    const visibleChoices = config.choices.slice(startIndex, endIndex);
    
    let output = `${prefix} ${config.message}\n`;
    output += 'Actions: (n) Create new branch, (m) Manage worktrees, (c) Clean up merged PRs, (q) Exit\n\n';
    
    visibleChoices.forEach((choice, index) => {
      const globalIndex = startIndex + index;
      const cursor = globalIndex === selectedIndex ? '❯' : ' ';
      output += `${cursor} ${choice.name}\n`;
    });
    
    return output;
  });
  
  return await branchSelectPrompt({
    message: 'Select a branch:',
    choices: branchChoices,
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

export async function selectWorktreeForManagement(worktrees: Array<{ branch: string; path: string }>): Promise<string | 'back'> {
  const choices = [
    ...worktrees.map((w, index) => ({
      name: `${index + 1}. ${w.branch}`,
      value: w.branch,
      description: w.path
    })),
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

export async function confirmContinue(message: string = 'Continue?'): Promise<boolean> {
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
      : target.hasUnpushedCommits
      ? 'Has unpushed commits'
      : false,
    checked: !target.hasUncommittedChanges && !target.hasUnpushedCommits
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