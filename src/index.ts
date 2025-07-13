#!/usr/bin/env node

import { 
  isGitRepository, 
  getAllBranches, 
  createBranch, 
  branchExists, 
  getRepositoryRoot,
  GitError 
} from './git.js';
import { 
  listWorktrees, 
  worktreeExists, 
  generateWorktreePath, 
  createWorktree,
  WorktreeError 
} from './worktree.js';
import { 
  launchClaudeCode, 
  isClaudeCodeAvailable, 
  ClaudeError 
} from './claude.js';
import { 
  selectBranch, 
  getNewBranchConfig, 
  selectBaseBranch, 
  confirmWorktreeCreation,
  confirmSkipPermissions
} from './ui/prompts.js';
import { 
  printWelcome, 
  printError, 
  printSuccess, 
  printInfo, 
  printWarning,
  formatBranchForDisplay 
} from './ui/display.js';
import { AppError } from './utils.js';
import { BranchChoice, WorktreeConfig } from './ui/types.js';

export async function main(): Promise<void> {
  try {
    printWelcome();

    // Check if current directory is a Git repository
    if (!(await isGitRepository())) {
      printError('Current directory is not a Git repository.');
      process.exit(1);
    }

    // Check if Claude Code is available
    if (!(await isClaudeCodeAvailable())) {
      printWarning('Claude Code CLI not found. Make sure it\'s installed and in your PATH.');
      printInfo('You can install it from: https://claude.ai/code');
    }

    // Get repository information
    const repoRoot = await getRepositoryRoot();
    const [branches, worktrees] = await Promise.all([
      getAllBranches(),
      listWorktrees()
    ]);

    // Create worktree lookup map
    const worktreeMap = new Map(worktrees.map(w => [w.branch, w]));

    // Format branches for display
    const branchChoices: BranchChoice[] = branches.map(branch => 
      formatBranchForDisplay(branch, worktreeMap.get(branch.name))
    );

    // Select branch or create new one
    const selectedValue = await selectBranch(branchChoices);

    let targetBranch: string;
    let isNewBranch = false;
    let baseBranch = 'main';

    if (selectedValue === '__create_new__') {
      // Create new branch flow
      const newBranchConfig = await getNewBranchConfig();
      targetBranch = newBranchConfig.branchName;
      isNewBranch = true;

      // Check if branch already exists
      if (await branchExists(targetBranch)) {
        printError(`Branch "${targetBranch}" already exists.`);
        process.exit(1);
      }

      // Select base branch
      baseBranch = await selectBaseBranch(branches);
      printInfo(`Creating new branch "${targetBranch}" from "${baseBranch}"`);
    } else {
      targetBranch = selectedValue;
      
      // Handle remote branch selection
      if (targetBranch.startsWith('origin/')) {
        const localBranchName = targetBranch.replace('origin/', '');
        
        if (await branchExists(localBranchName)) {
          targetBranch = localBranchName;
        } else {
          printInfo(`Creating local branch "${localBranchName}" from "${targetBranch}"`);
          await createBranch(localBranchName, targetBranch);
          targetBranch = localBranchName;
          isNewBranch = true;
        }
      }
    }

    // Check if worktree exists for the target branch
    const existingWorktreePath = await worktreeExists(targetBranch);

    // Ask about permissions before launching Claude Code
    const skipPermissions = await confirmSkipPermissions();

    if (existingWorktreePath) {
      printInfo(`Worktree already exists at: ${existingWorktreePath}`);
      await launchClaudeCode(existingWorktreePath, skipPermissions);
    } else {
      // Generate worktree path
      const worktreePath = await generateWorktreePath(repoRoot, targetBranch);

      // Confirm worktree creation
      const shouldCreate = await confirmWorktreeCreation(targetBranch, worktreePath);
      
      if (!shouldCreate) {
        printInfo('Operation cancelled.');
        process.exit(0);
      }

      // Create worktree configuration
      const worktreeConfig: WorktreeConfig = {
        branchName: targetBranch,
        worktreePath,
        repoRoot,
        isNewBranch,
        baseBranch
      };

      // Create worktree
      printInfo(`Creating worktree for "${targetBranch}"...`);
      await createWorktree(worktreeConfig);
      printSuccess(`Worktree created at: ${worktreePath}`);

      // Launch Claude Code
      await launchClaudeCode(worktreePath, skipPermissions);
    }

    printSuccess('Done!');

  } catch (error) {
    if (error instanceof GitError) {
      printError(`Git error: ${error.message}`);
    } else if (error instanceof WorktreeError) {
      printError(`Worktree error: ${error.message}`);
    } else if (error instanceof ClaudeError) {
      printError(`Claude Code error: ${error.message}`);
    } else if (error instanceof AppError) {
      printError(`Application error: ${error.message}`);
    } else {
      printError(`Unexpected error: ${error instanceof Error ? error.message : String(error)}`);
    }
    
    process.exit(1);
  }
}

// Run the application if this module is executed directly
if (import.meta.url === `file://${process.argv[1]}`) {
  main().catch(error => {
    console.error('Fatal error:', error);
    process.exit(1);
  });
}