import { select, input, confirm } from '@inquirer/prompts';
import { BranchInfo, BranchChoice, BranchType, NewBranchConfig } from './types.js';

export async function selectBranch(branches: BranchChoice[]): Promise<string> {
  return await select({
    message: 'Select a branch or create a new one:',
    choices: [
      {
        name: '🆕 Create new branch',
        value: '__create_new__',
        description: 'Create a new feature, hotfix, or release branch'
      },
      { name: '─'.repeat(50), value: '__separator__', disabled: true },
      ...branches
    ],
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
      if (/[\s\\\/\:\*\?\"\<\>\|]/.test(value.trim())) {
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