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
  isInWorktree,
  GitError,
  getCurrentVersion,
  calculateNewVersion,
  executeNpmVersionInWorktree
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
  confirmProceedWithoutPush,
  selectSession,
  selectClaudeExecutionMode,
  selectVersionBumpType
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
import { confirm } from '@inquirer/prompts';
import { isGitHubCLIAvailable, checkGitHubAuth } from './github.js';
import { CleanupTarget } from './ui/types.js';
import { AppError, setupExitHandlers, handleUserCancel } from './utils.js';
import { BranchInfo, WorktreeConfig } from './ui/types.js';
import { WorktreeInfo } from './worktree.js';
import { loadSession, saveSession, SessionData, getAllSessions } from './config/index.js';

function showHelp(): void {
  console.log(`
Claude Worktree Manager

Usage: claude-worktree [options]

Options:
  -c              Continue from the last session (automatically open the last used worktree)
  -r, --resume    Resume a session - interactively select from available sessions
  -h, --help      Show this help message

Description:
  Interactive Git worktree manager for Claude Code with graphical branch selection.
  
  Without options: Opens the interactive menu to select branches and manage worktrees.
  With -c option: Automatically continues from where you left off in the last session.
  With -r option: Shows a list of recent sessions to choose from and resume.
`);
}

export async function main(): Promise<void> {
  try {
    // Parse command line arguments
    const args = process.argv.slice(2);
    const continueLastSession = args.includes('-c');
    const resumeSession = args.includes('-r') || args.includes('--resume');
    const showHelpFlag = args.includes('-h') || args.includes('--help');

    // Show help if requested
    if (showHelpFlag) {
      showHelp();
      return;
    }

    // Setup graceful exit handlers
    setupExitHandlers();

    // Check if current directory is a Git repository
    if (!(await isGitRepository())) {
      printError('Current directory is not a Git repository.');
      process.exit(1);
    }
    
    // Check if running from a worktree directory
    if (await isInWorktree()) {
      printWarning('Running from a worktree directory is not recommended.');
      printInfo('Please run this command from the main repository root to avoid path issues.');
      printInfo('You can continue, but some operations may not work correctly.');
      const shouldContinue = await confirmContinue('Do you want to continue anyway?');
      if (!shouldContinue) {
        process.exit(0);
      }
    }

    // Check if Claude Code is available
    if (!(await isClaudeCodeAvailable())) {
      printWarning('Claude Code CLI not found. Make sure it\'s installed and in your PATH.');
      printInfo('You can install it from: https://claude.ai/code');
    }

    // Get repository root
    const repoRoot = await getRepositoryRoot();

    // Handle continue last session option
    if (continueLastSession) {
      const sessionData = await loadSession(repoRoot);
      if (sessionData && sessionData.lastWorktreePath) {
        printInfo(`Continuing last session: ${sessionData.lastBranch} (${sessionData.lastWorktreePath})`);
        
        // Check if worktree still exists
        if (await worktreeExists(sessionData.lastBranch!)) {
          const skipPermissions = await confirmSkipPermissions();
          
          try {
            await launchClaudeCode(sessionData.lastWorktreePath, { skipPermissions });
            await handlePostClaudeChanges(sessionData.lastWorktreePath);
          } catch (error) {
            if (error instanceof ClaudeError) {
              printError(`Failed to launch Claude Code: ${error.message}`);
            } else {
              printError(`Unexpected error: ${error instanceof Error ? error.message : String(error)}`);
            }
            await confirmContinue('Press enter to continue...');
          }
          
          return; // Exit after continuing session
        } else {
          printWarning(`Last session worktree no longer exists: ${sessionData.lastWorktreePath}`);
          printInfo('Falling back to normal flow...');
        }
      } else {
        printInfo('No previous session found. Starting normally...');
      }
    }

    // Handle resume session option
    if (resumeSession) {
      const allSessions = await getAllSessions();
      if (allSessions.length === 0) {
        printInfo('No previous sessions found. Starting normally...');
      } else {
        const selectedSession = await selectSession(allSessions);
        if (selectedSession && selectedSession.lastWorktreePath) {
          printInfo(`Resuming session: ${selectedSession.lastBranch} (${selectedSession.lastWorktreePath})`);
          
          // Check if worktree still exists
          if (await worktreeExists(selectedSession.lastBranch!)) {
            const skipPermissions = await confirmSkipPermissions();
            
            try {
              await launchClaudeCode(selectedSession.lastWorktreePath, { skipPermissions });
              await handlePostClaudeChanges(selectedSession.lastWorktreePath);
            } catch (error) {
              if (error instanceof ClaudeError) {
                printError(`Failed to launch Claude Code: ${error.message}`);
              } else {
                printError(`Unexpected error: ${error instanceof Error ? error.message : String(error)}`);
              }
              await confirmContinue('Press enter to continue...');
            }
            
            return; // Exit after resuming session
          } else {
            printWarning(`Selected session worktree no longer exists: ${selectedSession.lastWorktreePath}`);
            printInfo('Falling back to normal flow...');
          }
        } else {
          printInfo('No session selected. Starting normally...');
        }
      }
    }

    // Main application loop
    while (true) {
      try {
        // Get current repository state
        const [branches, worktrees] = await Promise.all([
          getAllBranches(),
          listAdditionalWorktrees()
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
      // Ask about execution mode
      const executionConfig = await selectClaudeExecutionMode();
      if (!executionConfig) {
        // User cancelled, return to main menu
        return true;
      }
      const { mode, skipPermissions } = executionConfig;
      
      try {
        // Save session data before launching Claude Code
        const sessionData: SessionData = {
          lastWorktreePath: worktreePath,
          lastBranch: targetBranch,
          timestamp: Date.now(),
          repositoryRoot: repoRoot
        };
        await saveSession(sessionData);
        
        await launchClaudeCode(worktreePath, { mode, skipPermissions });
        
        // Check for changes after Claude Code exits
        await handlePostClaudeChanges(worktreePath);
      } catch (error) {
        if (error instanceof ClaudeError) {
          printError(`Failed to launch Claude Code: ${error.message}`);
          if (error.message.includes('command not found')) {
            printInfo('Install with: pnpm add -g @anthropic-ai/claude-code');
          }
        } else {
          printError(`Unexpected error: ${error instanceof Error ? error.message : String(error)}`);
        }
        await confirmContinue('Press enter to continue...');
      }
    } else {
      printError('Claude Code is not available. Please install it first.');
      printInfo('Install with: pnpm add -g @anthropic-ai/claude-code');
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
    // まず、ブランチタイプのみを選択
    const { selectBranchType } = await import('./ui/prompts.js');
    const branchType = await selectBranchType();
    
    let targetBranch: string;
    let baseBranch: string;

    // リリースブランチの場合は特別な処理
    if (branchType === 'release') {
      // Git flowではリリースブランチはdevelopから分岐する
      const developBranch = branches.find(b => 
        b.type === 'local' && (b.name === 'develop' || b.name === 'dev')
      );
      
      if (!developBranch) {
        printError('No develop branch found. Release branches should be created from develop branch.');
        return true;
      }
      
      baseBranch = developBranch.name;
      printInfo(`Creating release branch from ${baseBranch}`);
      
      // 現在のバージョンを取得
      const currentVersion = await getCurrentVersion(repoRoot);
      
      // バージョンバンプタイプを選択
      const versionBump = await selectVersionBumpType(currentVersion);
      
      // 新しいバージョンを計算
      const newVersion = calculateNewVersion(currentVersion, versionBump);
      
      // リリースブランチ名を生成
      targetBranch = `release/${newVersion}`;
      printInfo(`Release branch will be: ${targetBranch}`);
    } else {
      // 通常のブランチの場合
      const { inputBranchName } = await import('./ui/prompts.js');
      const taskName = await inputBranchName(branchType);
      targetBranch = `${branchType}/${taskName}`;
      baseBranch = await selectBaseBranch(branches);
    }

    // Check if branch already exists
    if (await branchExists(targetBranch)) {
      printError(`Branch "${targetBranch}" already exists.`);
      if (await confirmContinue('Return to main menu?')) {
        return true;
      }
      return false;
    }

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
    
    // リリースブランチの場合、worktree作成後にnpm versionを実行
    if (branchType === 'release') {
      printInfo('Updating version in release branch...');
      try {
        const newVersion = targetBranch.replace('release/', '');
        await executeNpmVersionInWorktree(worktreePath, newVersion);
        printSuccess(`Version updated to ${newVersion} in release branch`);
      } catch (error) {
        printError(`Failed to update version: ${error instanceof Error ? error.message : String(error)}`);
        // エラーが発生してもworktreeは作成済みなので続行
      }
    }

    // Check if Claude Code is available before launching
    if (await isClaudeCodeAvailable()) {
      // Ask about execution mode
      const executionConfig = await selectClaudeExecutionMode();
      if (!executionConfig) {
        // User cancelled, return to main menu
        return true;
      }
      const { mode, skipPermissions } = executionConfig;
      
      try {
        // Save session data before launching Claude Code
        const sessionData: SessionData = {
          lastWorktreePath: worktreePath,
          lastBranch: targetBranch,
          timestamp: Date.now(),
          repositoryRoot: repoRoot
        };
        await saveSession(sessionData);
        
        await launchClaudeCode(worktreePath, { mode, skipPermissions });
        
        // Check for changes after Claude Code exits
        await handlePostClaudeChanges(worktreePath);
      } catch (error) {
        if (error instanceof ClaudeError) {
          printError(`Failed to launch Claude Code: ${error.message}`);
          if (error.message.includes('command not found')) {
            printInfo('Install with: pnpm add -g @anthropic-ai/claude-code');
          }
        } else {
          printError(`Unexpected error: ${error instanceof Error ? error.message : String(error)}`);
        }
        await confirmContinue('Press enter to continue...');
      }
    } else {
      printError('Claude Code is not available. Please install it first.');
      printInfo('Install with: pnpm add -g @anthropic-ai/claude-code');
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
          // Check if worktree is accessible
          if (worktree.isAccessible === false) {
            printError('Cannot open inaccessible worktree in Claude Code');
            printInfo(`Path: ${worktree.path}`);
            printInfo('This worktree was created in a different environment and is not accessible here.');
            await confirmContinue('Press enter to continue...');
            break;
          }
          
          // Check if Claude Code is available before launching
          if (await isClaudeCodeAvailable()) {
            // Ask about execution mode
      const executionConfig = await selectClaudeExecutionMode();
      if (!executionConfig) {
        // User cancelled, return to main menu
        return true;
      }
      const { mode, skipPermissions } = executionConfig;
            
            try {
              // Save session data before launching Claude Code
              const sessionData: SessionData = {
                lastWorktreePath: worktree.path,
                lastBranch: worktree.branch,
                timestamp: Date.now(),
                repositoryRoot: await getRepositoryRoot()
              };
              await saveSession(sessionData);
              
              await launchClaudeCode(worktree.path, { mode, skipPermissions });
            } catch (error) {
              if (error instanceof ClaudeError) {
                printError(`Failed to launch Claude Code: ${error.message}`);
                if (error.message.includes('command not found')) {
                  printInfo('Install with: pnpm add -g @anthropic-ai/claude-code');
                }
              } else {
                printError(`Unexpected error: ${error instanceof Error ? error.message : String(error)}`);
              }
              await confirmContinue('Press enter to continue...');
            }
          } else {
            printError('Claude Code is not available. Please install it first.');
            printInfo('Install with: pnpm add -g @anthropic-ai/claude-code');
            await confirmContinue('Press enter to continue...');
          }
          return true; // Return to main menu after opening
          
        case 'remove':
          if (worktree.isAccessible === false) {
            // Special handling for inaccessible worktrees
            const shouldRemove = await confirm({
              message: 'This worktree is inaccessible. Do you want to remove it from Git\'s records?',
              default: false
            });
            if (shouldRemove) {
              await removeWorktree(worktree.path, true); // Force removal
              printSuccess(`Worktree record removed: ${worktree.path}`);
              // Update worktrees list
              const index = worktrees.indexOf(worktree);
              worktrees.splice(index, 1);
            }
          } else {
            if (await confirmWorktreeRemoval(worktree.path)) {
              await removeWorktree(worktree.path);
              printSuccess(`Worktree removed: ${worktree.path}`);
              // Update worktrees list
              const index = worktrees.indexOf(worktree);
              worktrees.splice(index, 1);
            }
          }
          break;
          
        case 'remove-branch':
          if (worktree.isAccessible === false) {
            // Special handling for inaccessible worktrees
            const shouldRemove = await confirm({
              message: 'This worktree is inaccessible. Do you want to remove it from Git\'s records and delete the branch?',
              default: false
            });
            if (shouldRemove) {
              await removeWorktree(worktree.path, true); // Force removal
              printSuccess(`Worktree record removed: ${worktree.path}`);
              
              if (await confirmBranchRemoval(worktree.branch)) {
                await deleteBranch(worktree.branch, true); // Force delete
                printSuccess(`Branch deleted: ${worktree.branch}`);
              }
              
              // Update worktrees list
              const index = worktrees.indexOf(worktree);
              worktrees.splice(index, 1);
            }
          } else {
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
      await confirmContinue('Press enter to continue...');
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