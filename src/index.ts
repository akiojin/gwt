#!/usr/bin/env node

import { 
  isGitRepository, 
  getAllBranches, 
  createBranch, 
  branchExists, 
  getRepositoryRoot,
  deleteBranch,
  deleteRemoteBranch,
  hasUncommittedChanges,
  showStatus,
  stashChanges,
  discardAllChanges,
  commitChanges,
  fetchAllRemotes,
  pushBranchToRemote,
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
  confirmCleanup,
  confirmRemoteBranchDeletion,
  confirmPushUnpushedCommits,
  confirmProceedWithoutPush
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
      if (error.cause && error.cause instanceof Error) {
        printError(`Cause: ${error.cause.message}`);
      }
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
    // Check if this is a remote branch
    const isRemoteBranch = branchName.startsWith('origin/');
    let localBranchName = branchName;
    let targetBranch = branchName;
    
    if (isRemoteBranch) {
      // Extract local branch name from remote branch
      localBranchName = branchName.replace(/^origin\//, '');
      targetBranch = localBranchName;
    }
    
    // Check if worktree exists (using local branch name)
    let worktreePath = await worktreeExists(targetBranch);
    
    if (worktreePath) {
      printInfo(`Opening existing worktree: ${worktreePath}`);
    } else {
      // Create new worktree
      worktreePath = await generateWorktreePath(repoRoot, targetBranch);
      
      if (!(await confirmWorktreeCreation(targetBranch, worktreePath))) {
        printInfo('Operation cancelled.');
        return true; // Continue to main menu
      }

      let isNewBranch = false;
      let baseBranch = targetBranch;
      
      if (isRemoteBranch) {
        // Check if local branch exists
        const localExists = await branchExists(localBranchName);
        if (!localExists) {
          // Need to create new local branch from remote
          isNewBranch = true;
          baseBranch = branchName; // Use full remote branch name as base
        }
      }

      const worktreeConfig: WorktreeConfig = {
        branchName: targetBranch,
        worktreePath,
        repoRoot,
        isNewBranch,
        baseBranch
      };

      printInfo(`Creating worktree for "${targetBranch}"...`);
      await createWorktree(worktreeConfig);
      printSuccess(`Worktree created at: ${worktreePath}`);
    }

    // Check if Claude Code is available before launching
    if (await isClaudeCodeAvailable()) {
      // Ask about permissions and launch Claude Code
      const skipPermissions = await confirmSkipPermissions();
      
      try {
        await launchClaudeCode(worktreePath, skipPermissions);
        
        // Check for changes after Claude Code exits
        await handlePostClaudeChanges(worktreePath);
      } catch (error) {
        if (error instanceof ClaudeError) {
          printError(`Failed to launch Claude Code: ${error.message}`);
          if (error.message.includes('command not found')) {
            printInfo('Install with: npm install -g @anthropic-ai/claude-code');
          }
        } else {
          printError(`Unexpected error: ${error instanceof Error ? error.message : String(error)}`);
        }
        await confirmContinue('Press enter to continue...');
      }
    } else {
      printError('Claude Code is not available. Please install it first.');
      printInfo('Install with: npm install -g @anthropic-ai/claude-code');
      await confirmContinue('Press enter to continue...');
    }
    
    // After handling changes, return to main menu
    return true;

  } catch (error) {
    printError(`Failed to handle branch selection: ${error instanceof Error ? error.message : String(error)}`);
    await confirmContinue('Press enter to continue...');
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

    // Check if Claude Code is available before launching
    if (await isClaudeCodeAvailable()) {
      // Launch Claude Code
      const skipPermissions = await confirmSkipPermissions();
      
      try {
        await launchClaudeCode(worktreePath, skipPermissions);
        
        // Check for changes after Claude Code exits
        await handlePostClaudeChanges(worktreePath);
      } catch (error) {
        if (error instanceof ClaudeError) {
          printError(`Failed to launch Claude Code: ${error.message}`);
          if (error.message.includes('command not found')) {
            printInfo('Install with: npm install -g @anthropic-ai/claude-code');
          }
        } else {
          printError(`Unexpected error: ${error instanceof Error ? error.message : String(error)}`);
        }
        await confirmContinue('Press enter to continue...');
      }
    } else {
      printError('Claude Code is not available. Please install it first.');
      printInfo('Install with: npm install -g @anthropic-ai/claude-code');
      await confirmContinue('Press enter to continue...');
    }
    
    return true;

  } catch (error) {
    printError(`Failed to create new branch: ${error instanceof Error ? error.message : String(error)}`);
    await confirmContinue('Press enter to continue...');
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
          // Check if Claude Code is available before launching
          if (await isClaudeCodeAvailable()) {
            const skipPermissions = await confirmSkipPermissions();
            
            try {
              await launchClaudeCode(worktree.path, skipPermissions);
            } catch (error) {
              if (error instanceof ClaudeError) {
                printError(`Failed to launch Claude Code: ${error.message}`);
                if (error.message.includes('command not found')) {
                  printInfo('Install with: npm install -g @anthropic-ai/claude-code');
                }
              } else {
                printError(`Unexpected error: ${error instanceof Error ? error.message : String(error)}`);
              }
              await confirmContinue('Press enter to continue...');
            }
          } else {
            printError('Claude Code is not available. Please install it first.');
            printInfo('Install with: npm install -g @anthropic-ai/claude-code');
            await confirmContinue('Press enter to continue...');
          }
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
    await confirmContinue('Press enter to continue...');
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

    // Check if there are branches with unpushed commits and ask about pushing
    const shouldPushUnpushed = await confirmPushUnpushedCommits(selectedTargets);
    
    // Ask about remote branch deletion
    const deleteRemoteBranches = await confirmRemoteBranchDeletion(selectedTargets);

    // Perform cleanup
    const results: Array<{ target: CleanupTarget; success: boolean; error?: string }> = [];

    for (const target of selectedTargets) {
      try {
        // Push unpushed commits if requested and needed (only for worktree targets)
        if (shouldPushUnpushed && target.hasUnpushedCommits && target.cleanupType === 'worktree-and-branch') {
          printInfo(`Pushing unpushed commits in branch: ${target.branch}`);
          try {
            await pushBranchToRemote(target.worktreePath!, target.branch);
            printSuccess(`Successfully pushed changes for branch: ${target.branch}`);
          } catch (error) {
            printWarning(`Failed to push branch ${target.branch}: ${error instanceof Error ? error.message : String(error)}`);
            
            // Ask user if they want to proceed without pushing
            if (!(await confirmProceedWithoutPush(target.branch))) {
              printInfo(`Skipping deletion of branch: ${target.branch}`);
              continue; // Skip this target
            }
          }
        }
        
        // Handle different cleanup types
        if (target.cleanupType === 'worktree-and-branch') {
          printInfo(`Removing worktree: ${target.worktreePath}`);
          await removeWorktree(target.worktreePath!, true); // Force remove
          
          printInfo(`Deleting local branch: ${target.branch}`);
          await deleteBranch(target.branch, true); // Force delete
        } else if (target.cleanupType === 'branch-only') {
          printInfo(`Deleting local branch: ${target.branch}`);
          await deleteBranch(target.branch, true); // Force delete
        }
        
        if (deleteRemoteBranches && target.hasRemoteBranch) {
          printInfo(`Deleting remote branch: origin/${target.branch}`);
          try {
            await deleteRemoteBranch(target.branch);
            printSuccess(`Successfully deleted remote branch: origin/${target.branch}`);
          } catch (error) {
            // リモートブランチの削除に失敗してもローカルの削除は成功として扱う
            printWarning(`Failed to delete remote branch: ${error instanceof Error ? error.message : String(error)}`);
          }
        }
        
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
    await confirmContinue('Press enter to continue...');
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
    await confirmContinue('Press enter to continue...');
  }
}

// Run the application if this module is executed directly
if (import.meta.url === `file://${process.argv[1]}`) {
  main().catch(error => {
    console.error('Fatal error:', error);
    process.exit(1);
  });
}