#!/usr/bin/env node

import { 
  isGitRepository, 
  getAllBranches, 
  createBranch, 
  branchExists, 
  getRepositoryRoot,
  deleteBranch,
  hasUncommittedChanges,
  showStatus,
  stashChanges,
  discardAllChanges,
  commitChanges,
  fetchAllRemotes,
  GitError 
} from './git.js';
import { 
  listAdditionalWorktrees,
  worktreeExists, 
  generateWorktreePath, 
  createWorktree,
  removeWorktree,
  getMergedPRWorktrees,
  WorktreeError 
} from './worktree.js';
import { 
  launchClaudeCode, 
  isClaudeCodeAvailable, 
  ClaudeError 
} from './claude.js';
import { 
  selectFromTable,
  getNewBranchConfig, 
  selectBaseBranch, 
  confirmWorktreeCreation,
  confirmSkipPermissions,
  selectWorktreeForManagement,
  selectWorktreeAction,
  confirmWorktreeRemoval,
  confirmBranchRemoval,
  selectChangesAction,
  inputCommitMessage,
  confirmDiscardChanges,
  confirmContinue,
  selectCleanupTargets,
  confirmCleanup
} from './ui/prompts.js';
import { 
  displayBranchTable,
  printError, 
  printSuccess, 
  printInfo, 
  printWarning,
  printExit,
  printStatistics,
  displayCleanupTargets,
  displayCleanupResults
} from './ui/display.js';
import { createBranchTable } from './ui/table.js';
import chalk from 'chalk';
import { isGitHubCLIAvailable, checkGitHubAuth } from './github.js';
import { CleanupTarget } from './ui/types.js';
import { AppError, setupExitHandlers, handleUserCancel } from './utils.js';
import { BranchInfo, WorktreeConfig } from './ui/types.js';
import { WorktreeInfo } from './worktree.js';

export async function main(): Promise<void> {
  try {
    // Setup graceful exit handlers
    setupExitHandlers();

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

    // Main application loop
    while (true) {
      try {
        // Get current repository state
        const [branches, worktrees, repoRoot] = await Promise.all([
          getAllBranches(),
          listAdditionalWorktrees(),
          getRepositoryRoot()
        ]);

        // Create and display table
        const choices = await createBranchTable(branches, worktrees);
        await displayBranchTable();

        // Get user selection with statistics
        const selection = await selectFromTable(choices, { branches, worktrees });

        // Handle selection
        const shouldContinue = await handleSelection(selection, branches, worktrees, repoRoot);
        
        if (!shouldContinue) {
          break;
        }

      } catch (error) {
        handleUserCancel(error);
      }
    }

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

async function handleSelection(
  selection: string,
  branches: BranchInfo[],
  worktrees: WorktreeInfo[],
  repoRoot: string
): Promise<boolean> {
  
  switch (selection) {
    case '__exit__':
      printExit();
      return false;

    case '__create_new__':
      return await handleCreateNewBranch(branches, repoRoot);

    case '__manage_worktrees__':
      return await handleManageWorktrees(worktrees);

    case '__cleanup_prs__':
      return await handleCleanupMergedPRs();

    default:
      // Handle branch selection
      return await handleBranchSelection(selection, repoRoot);
  }
}

async function handleBranchSelection(branchName: string, repoRoot: string): Promise<boolean> {
  try {
    // Check if worktree exists
    let worktreePath = await worktreeExists(branchName);
    
    if (worktreePath) {
      printInfo(`Opening existing worktree: ${worktreePath}`);
    } else {
      // Create new worktree
      worktreePath = await generateWorktreePath(repoRoot, branchName);
      
      if (!(await confirmWorktreeCreation(branchName, worktreePath))) {
        printInfo('Operation cancelled.');
        return true; // Continue to main menu
      }

      const worktreeConfig: WorktreeConfig = {
        branchName,
        worktreePath,
        repoRoot,
        isNewBranch: false,
        baseBranch: branchName
      };

      printInfo(`Creating worktree for "${branchName}"...`);
      await createWorktree(worktreeConfig);
      printSuccess(`Worktree created at: ${worktreePath}`);
    }

    // Ask about permissions and launch Claude Code
    const skipPermissions = await confirmSkipPermissions();
    await launchClaudeCode(worktreePath, skipPermissions);
    
    // Check for changes after Claude Code exits
    await handlePostClaudeChanges(worktreePath);
    
    // After handling changes, return to main menu
    return true;

  } catch (error) {
    printError(`Failed to handle branch selection: ${error instanceof Error ? error.message : String(error)}`);
    return true;
  }
}

async function handleCreateNewBranch(branches: BranchInfo[], repoRoot: string): Promise<boolean> {
  try {
    const newBranchConfig = await getNewBranchConfig();
    const targetBranch = newBranchConfig.branchName;

    // Check if branch already exists
    if (await branchExists(targetBranch)) {
      printError(`Branch "${targetBranch}" already exists.`);
      if (await confirmContinue('Return to main menu?')) {
        return true;
      }
      return false;
    }

    // Select base branch
    const baseBranch = await selectBaseBranch(branches);
    printInfo(`Creating new branch "${targetBranch}" from "${baseBranch}"`);

    // Create worktree path
    const worktreePath = await generateWorktreePath(repoRoot, targetBranch);
    
    if (!(await confirmWorktreeCreation(targetBranch, worktreePath))) {
      printInfo('Operation cancelled.');
      return true;
    }

    // Create worktree configuration
    const worktreeConfig: WorktreeConfig = {
      branchName: targetBranch,
      worktreePath,
      repoRoot,
      isNewBranch: true,
      baseBranch
    };

    // Create worktree
    printInfo(`Creating worktree for "${targetBranch}"...`);
    await createWorktree(worktreeConfig);
    printSuccess(`Worktree created at: ${worktreePath}`);

    // Launch Claude Code
    const skipPermissions = await confirmSkipPermissions();
    await launchClaudeCode(worktreePath, skipPermissions);
    
    // Check for changes after Claude Code exits
    await handlePostClaudeChanges(worktreePath);
    
    return true;

  } catch (error) {
    printError(`Failed to create new branch: ${error instanceof Error ? error.message : String(error)}`);
    return true;
  }
}

async function handleManageWorktrees(worktrees: WorktreeInfo[]): Promise<boolean> {
  try {
    if (worktrees.length === 0) {
      printInfo('No worktrees found.');
      if (await confirmContinue('Return to main menu?')) {
        return true;
      }
      return false;
    }

    while (true) {
      const worktreeChoices = worktrees.map(w => ({ branch: w.branch, path: w.path }));
      const selectedWorktree = await selectWorktreeForManagement(worktreeChoices);
      
      if (selectedWorktree === 'back') {
        return true; // Return to main menu
      }

      const worktree = worktrees.find(w => w.branch === selectedWorktree);
      if (!worktree) {
        printError('Worktree not found.');
        continue;
      }

      const action = await selectWorktreeAction();
      
      switch (action) {
        case 'open':
          const skipPermissions = await confirmSkipPermissions();
          await launchClaudeCode(worktree.path, skipPermissions);
          return true; // Return to main menu after opening
          
        case 'remove':
          if (await confirmWorktreeRemoval(worktree.path)) {
            await removeWorktree(worktree.path);
            printSuccess(`Worktree removed: ${worktree.path}`);
            // Update worktrees list
            const index = worktrees.indexOf(worktree);
            worktrees.splice(index, 1);
          }
          break;
          
        case 'remove-branch':
          if (await confirmWorktreeRemoval(worktree.path)) {
            await removeWorktree(worktree.path);
            printSuccess(`Worktree removed: ${worktree.path}`);
            
            if (await confirmBranchRemoval(worktree.branch)) {
              await deleteBranch(worktree.branch, true); // Force delete
              printSuccess(`Branch deleted: ${worktree.branch}`);
            }
            
            // Update worktrees list
            const index = worktrees.indexOf(worktree);
            worktrees.splice(index, 1);
          }
          break;
          
        case 'back':
          continue; // Continue worktree management loop
      }
    }

  } catch (error) {
    printError(`Failed to manage worktrees: ${error instanceof Error ? error.message : String(error)}`);
    return true;
  }
}

async function handleCleanupMergedPRs(): Promise<boolean> {
  try {
    // Check if GitHub CLI is available
    if (!(await isGitHubCLIAvailable())) {
      printError('GitHub CLI is not installed. Please install it to use this feature.');
      printInfo('Install GitHub CLI: https://cli.github.com/');
      return true;
    }

    // Check if authenticated
    if (!(await checkGitHubAuth())) {
      return true;
    }

    printInfo('Fetching latest changes from remote...');
    await fetchAllRemotes();

    printInfo('Checking for merged pull requests...');
    const cleanupTargets = await getMergedPRWorktrees();

    if (cleanupTargets.length === 0) {
      console.log(chalk.green('✨ すべてクリーンです！クリーンアップが必要なworktreeはありません。'));
      return true;
    }

    // Display targets
    displayCleanupTargets(cleanupTargets);

    // Select targets to clean up
    const selectedTargets = await selectCleanupTargets(cleanupTargets);

    if (selectedTargets.length === 0) {
      console.log(chalk.yellow('🚫 クリーンアップをキャンセルしました。'));
      return true;
    }

    // Confirm cleanup
    if (!(await confirmCleanup(selectedTargets))) {
      console.log(chalk.yellow('🚫 クリーンアップをキャンセルしました。'));
      return true;
    }

    // Perform cleanup
    const results: Array<{ target: CleanupTarget; success: boolean; error?: string }> = [];

    for (const target of selectedTargets) {
      try {
        printInfo(`Removing worktree: ${target.worktreePath}`);
        await removeWorktree(target.worktreePath, true); // Force remove
        
        printInfo(`Deleting branch: ${target.branch}`);
        await deleteBranch(target.branch, true); // Force delete
        
        results.push({ target, success: true });
      } catch (error) {
        results.push({ 
          target, 
          success: false, 
          error: error instanceof Error ? error.message : String(error) 
        });
      }
    }

    // Display results
    displayCleanupResults(results);

    return true;

  } catch (error) {
    printError(`Failed to cleanup merged PRs: ${error instanceof Error ? error.message : String(error)}`);
    return true;
  }
}

async function handlePostClaudeChanges(worktreePath: string): Promise<void> {
  try {
    // Check if there are uncommitted changes
    if (!(await hasUncommittedChanges(worktreePath))) {
      return; // No changes, nothing to do
    }

    while (true) {
      const action = await selectChangesAction();
      
      switch (action) {
        case 'status':
          const status = await showStatus(worktreePath);
          console.log('\n' + status + '\n');
          await confirmContinue('Press enter to continue...');
          break;
          
        case 'commit':
          const commitMessage = await inputCommitMessage();
          await commitChanges(worktreePath, commitMessage);
          printSuccess('Changes committed successfully!');
          return;
          
        case 'stash':
          await stashChanges(worktreePath, 'Stashed by Claude Worktree Manager');
          printSuccess('Changes stashed successfully!');
          return;
          
        case 'discard':
          if (await confirmDiscardChanges()) {
            await discardAllChanges(worktreePath);
            printSuccess('All changes discarded.');
            return;
          }
          break;
          
        case 'continue':
          return;
      }
    }
  } catch (error) {
    printError(`Failed to handle changes: ${error instanceof Error ? error.message : String(error)}`);
  }
}

// Run the application if this module is executed directly
if (import.meta.url === `file://${process.argv[1]}`) {
  main().catch(error => {
    console.error('Fatal error:', error);
    process.exit(1);
  });
}