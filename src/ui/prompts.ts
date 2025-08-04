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
        // 最上部で停止（ループしない）
        if (selectedIndex > 0) {
          setSelectedIndex(selectedIndex - 1);
        }
        return;
      }
      if (key.name === 'down' || key.name === 'j') {
        // 選択可能な項目数に基づいて制限
        const selectableChoices = config.choices.filter(c => 
          c.value !== '__header__' && 
          c.value !== '__separator__' &&
          !c.disabled
        );
        // 最下部で停止（ループしない）
        if (selectedIndex < selectableChoices.length - 1) {
          setSelectedIndex(selectedIndex + 1);
        }
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

  ];

  try {
    return await select({
      message: 'Select worktree to manage (q to go back):',
      choices,
      pageSize: 15
    });
  } catch {
    // Handle q key - return 'back' to maintain compatibility
    return 'back';
  }
}

export async function selectWorktreeAction(): Promise<'open' | 'remove' | 'remove-branch' | 'back'> {
  try {
    return await select({
      message: 'What would you like to do (q to go back)?',
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
        }
      ]
    });
  } catch {
    // Handle q key - return 'back' to maintain compatibility
    return 'back';
  }
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

  console.log('\n' + chalk.bold.cyan('Recent Claude Code Sessions'));
  console.log(chalk.gray('Select a session to resume:\n'));

  // Collect enhanced session information with categorization
  const categorizedSessions: CategorizedSession[] = [];
  
  for (let index = 0; index < sessions.length; index++) {
    const session = sessions[index];
    if (!session) continue;
    
    if (!session.lastWorktreePath || !session.lastBranch) {
      // Create a fallback category for incomplete sessions
      const fallbackInfo: import('../git.js').EnhancedSessionInfo = {
        hasUncommittedChanges: false,
        uncommittedChangesCount: 0,
        hasUnpushedCommits: false,
        unpushedCommitsCount: 0,
        latestCommitMessage: null,
        branchType: 'other'
      };
      
      categorizedSessions.push({
        session,
        sessionInfo: fallbackInfo,
        category: categorizeSession(fallbackInfo),
        index
      });
      continue;
    }

    try {
      const { getEnhancedSessionInfo } = await import('../git.js');
      const sessionInfo = await getEnhancedSessionInfo(session.lastWorktreePath, session.lastBranch);
      const category = categorizeSession(sessionInfo);
      
      categorizedSessions.push({
        session,
        sessionInfo,
        category,
        index
      });
    } catch {
      // Fallback for sessions where enhanced info is not available
      const fallbackInfo: import('../git.js').EnhancedSessionInfo = {
        hasUncommittedChanges: false,
        uncommittedChangesCount: 0,
        hasUnpushedCommits: false,
        unpushedCommitsCount: 0,
        latestCommitMessage: null,
        branchType: 'other'
      };
      
      categorizedSessions.push({
        session,
        sessionInfo: fallbackInfo,
        category: categorizeSession(fallbackInfo),
        index
      });
    }
  }

  // Group and sort sessions
  const groupedSessions = groupAndSortSessions(categorizedSessions);
  
  // Create choices with grouping
  const groupedChoices = createGroupedChoices(groupedSessions);
  
  // No cancel option - use q key to go back

  let selectedIndex;
  try {
    selectedIndex = await select({
      message: 'Select session (q to go back):',
      choices: groupedChoices,
      pageSize: 12
    });
  } catch {
    // Handle q key - user wants to go back
    return null;
  }

  const index = parseInt(selectedIndex);
  return sessions[index] ?? null;
}

/**
 * Select Claude Code conversation from history
 */
export async function selectClaudeConversation(worktreePath: string): Promise<import('../claude-history.js').ClaudeConversation | null> {
  try {
    const { getConversationsForProject, isClaudeHistoryAvailable } = await import('../claude-history.js');
    
    // Check if Claude Code history is available
    if (!(await isClaudeHistoryAvailable())) {
      console.log(chalk.yellow('⚠️  Claude Code history not found on this system'));
      console.log(chalk.gray('   Using standard Claude Code resume functionality instead...'));
      return null;
    }

    console.log('\n' + chalk.bold.cyan('🔄 Resume Claude Code Conversation'));
    console.log(chalk.gray('Select a conversation to resume:\n'));

    // Get conversations for the current project
    const conversations = await getConversationsForProject(worktreePath);
    
    if (conversations.length === 0) {
      console.log(chalk.yellow('📝 No conversations found for this project'));
      console.log(chalk.gray('   Starting a new conversation instead...'));
      return null;
    }

    // Categorize conversations by recency
    const categorizedConversations = categorizeConversationsByActivity(conversations);
    
    // Create grouped choices
    const choices = createConversationChoices(categorizedConversations);
    
    // No cancel option - use q key to go back

    // Single selection prompt
    let selectedValue;
    try {
      selectedValue = await select({
        message: 'Choose conversation to resume (q to go back):',
        choices: choices,
        pageSize: 15
      });
    } catch {
      // Handle q key - user wants to go back
      return null;
    }

    const selectedIndex = parseInt(selectedValue);
    const selectedConversation = conversations[selectedIndex] || null;
    
    if (!selectedConversation) {
      return null;
    }

    // Clear screen before showing preview
    console.clear();
    
    // Show enhanced preview
    console.log(chalk.bold.cyan('📖 Conversation Preview'));
    console.log(chalk.gray('─'.repeat(Math.min(80, process.stdout.columns || 80))));
    console.log();
    
    const { getDetailedConversation } = await import('../claude-history.js');
    const detailed = await getDetailedConversation(selectedConversation);
    if (detailed) {
      displayConversationPreview(detailed.messages);
    }
    
    console.log();
    console.log(chalk.gray('─'.repeat(Math.min(80, process.stdout.columns || 80))));

    // Simplified action selection - use q to go back
    let action;
    try {
      action = await select({
        message: 'What would you like to do (q to go back)?',
        choices: [
          {
            name: chalk.green(`✅ Resume "${selectedConversation.title}"`),
            value: 'resume'
          },
          {
            name: chalk.blue('📋 View more messages'),
            value: 'view_more'
          }
        ]
      });
    } catch {
      // Handle q key - go back to conversation selection
      console.clear();
      return await selectClaudeConversation(worktreePath);
    }
    
    switch (action) {
      case 'resume':
        return selectedConversation;
      case 'view_more': {
        // Show extended preview
        console.clear();
        console.log(chalk.bold.cyan('📖 Extended Conversation History'));
        console.log(chalk.gray('─'.repeat(Math.min(80, process.stdout.columns || 80))));
        console.log();
        
        if (detailed) {
          displayExtendedConversationPreview(detailed.messages);
        }
        
        console.log();
        console.log(chalk.gray('─'.repeat(Math.min(80, process.stdout.columns || 80))));
        
        const resumeAfterExtended = await confirm({
          message: `Resume "${selectedConversation.title}"?`,
          default: true
        });
        
        if (resumeAfterExtended) {
          return selectedConversation;
        } else {
          console.clear();
          return await selectClaudeConversation(worktreePath);
        }
      }
      default:
        return null;
    }
  } catch {
    console.error(chalk.red('Failed to load Claude Code conversations:'));
    console.log(chalk.gray('Using standard Claude Code resume functionality instead...'));
    return null;
  }
}

/**
 * Display conversation messages with scrollable interface
 */
export async function displayConversationMessages(conversation: import('../claude-history.js').ClaudeConversation): Promise<boolean> {
  try {
    const { getDetailedConversation } = await import('../claude-history.js');
    const detailedConversation = await getDetailedConversation(conversation);
    
    if (!detailedConversation || !detailedConversation.messages) {
      console.log(chalk.red('Unable to load conversation messages'));
      return false;
    }

    console.clear();
    console.log(chalk.bold.cyan(`📖 ${conversation.title}`));
    console.log(chalk.gray(`${conversation.messageCount} messages • ${formatTimeAgo(conversation.lastActivity)}`));
    console.log(chalk.gray('─'.repeat(80)));
    console.log();

    // Create scrollable message viewer
    return await createMessageViewer(detailedConversation.messages);
  } catch {
    console.error(chalk.red('Failed to display conversation messages:'));
    return false;
  }
}

/**
 * Create scrollable message viewer component
 */
async function createMessageViewer(messages: import('../claude-history.js').ClaudeMessage[]): Promise<boolean> {
  console.clear();
  console.log(chalk.bold.cyan(`📖 Conversation History (${messages.length} messages)`));
  console.log(chalk.gray('─'.repeat(80)));
  console.log();
  
  // Show recent messages (last 10)
  const recentMessages = messages.slice(-10);
  
  recentMessages.forEach((message) => {
    const isUser = message.role === 'user';
    const roleSymbol = isUser ? '>' : '⏺';
    const roleColor = isUser ? chalk.blue : chalk.cyan;
    
    // Format message content
    let content = '';
    if (typeof message.content === 'string') {
      content = message.content;
    } else if (Array.isArray(message.content)) {
      content = message.content.map(item => item.text || '').join(' ');
    }
    
    // Handle special content types
    let displayContent = content;
    let toolInfo = '';
    
    if (content.startsWith('🔧 Used tool:')) {
      const toolName = content.replace('🔧 Used tool: ', '');
      toolInfo = chalk.yellow(`[Tool: ${toolName}]`);
      displayContent = ''; // Don't show content for tool calls
    } else if (content.length > 60) {
      // Truncate long messages
      displayContent = content.substring(0, 57) + '...';
    }
    
    // Format like Claude Code
    const roleDisplay = roleColor(roleSymbol);
    
    // Display the message with Claude Code formatting
    if (toolInfo) {
      console.log(`${roleDisplay} ${toolInfo}`);
    } else if (displayContent.trim()) {
      console.log(`${roleDisplay} ${displayContent}`);
    }
    
    // Add spacing between messages like Claude Code
    console.log();
  });
  
  if (messages.length > 10) {
    console.log();
    console.log(chalk.gray(`... and ${messages.length - 10} more messages above`));
  }
  
  console.log();
  console.log(chalk.gray('─'.repeat(80)));
  console.log();
  
  // Simple confirmation
  return await confirm({
    message: 'Resume this conversation?',
    default: true
  });
}

/**
 * Display conversation preview (ccresume style)
 */
function displayConversationPreview(messages: import('../claude-history.js').ClaudeMessage[]): void {
  // Get terminal height and calculate available space for messages
  const terminalHeight = process.stdout.rows || 24; // Default to 24 if unavailable
  const headerLines = 3; // Title + separator + empty line
  const footerLines = 3; // Empty line + separator + confirmation prompt
  const availableLines = Math.max(10, terminalHeight - headerLines - footerLines);
  
  // Show more messages based on available terminal space
  // Start with recent messages and work backwards
  const messagesToShow = Math.min(messages.length, Math.floor(availableLines / 2)); // Estimate 2 lines per message on average
  const recentMessages = messages.slice(-messagesToShow);
  
  recentMessages.forEach((message) => {
    const isUser = message.role === 'user';
    const roleSymbol = isUser ? '>' : '⏺';
    const roleColor = isUser ? chalk.blue : chalk.cyan;
    
    // Format message content
    let content = '';
    if (typeof message.content === 'string') {
      content = message.content;
    } else if (Array.isArray(message.content)) {
      content = message.content.map(item => item.text || '').join(' ');
    }
    
    // Handle special content types
    let displayContent = content;
    
    if (content.startsWith('🔧 Used tool:')) {
      const toolName = content.replace('🔧 Used tool: ', '');
      displayContent = chalk.yellow(`[Tool: ${toolName}]`);
    } else {
      // Don't truncate as aggressively - use more terminal width
      const terminalWidth = process.stdout.columns || 80;
      const maxContentWidth = terminalWidth - 15; // Account for role label and spacing
      
      if (content.length > maxContentWidth) {
        // For long content, show more but still truncate if needed
        displayContent = content.substring(0, maxContentWidth - 3) + '...';
      } else {
        displayContent = content;
      }
      
      // Handle multi-line content - show first few lines
      const lines = displayContent.split('\n');
      if (lines.length > 3) {
        displayContent = lines.slice(0, 3).join('\n') + '\n' + chalk.gray(`... (${lines.length - 3} more lines)`);
      }
    }
    
    // Format like Claude Code
    const roleDisplay = roleColor(roleSymbol);
    
    // Handle multi-line display
    const contentLines = displayContent.split('\n');
    contentLines.forEach((line, index) => {
      if (index === 0) {
        console.log(`${roleDisplay} ${line}`);
      } else {
        // Indent continuation lines
        console.log(`${' '.repeat(roleDisplay.length - 8)} ${line}`); // Account for ANSI color codes
      }
    });
  });
  
  if (messages.length > messagesToShow) {
    console.log(chalk.gray(`... and ${messages.length - messagesToShow} more messages above`));
  }
  
  // Add some spacing if we have room
  if (messagesToShow < availableLines / 3) {
    console.log();
  }
}

/**
 * Display extended conversation preview with more messages
 */
function displayExtendedConversationPreview(messages: import('../claude-history.js').ClaudeMessage[]): void {
  // Get terminal height and use most of it for extended preview
  const terminalHeight = process.stdout.rows || 24;
  const headerLines = 3; // Title + separator + empty line
  const footerLines = 4; // Empty line + separator + confirmation prompt + extra space
  const availableLines = Math.max(15, terminalHeight - headerLines - footerLines);
  
  // Show many more messages for extended preview - aim to fill most of the screen
  const messagesToShow = Math.min(messages.length, Math.floor(availableLines * 0.8)); // Use 80% of available lines
  const recentMessages = messages.slice(-messagesToShow);
  
  recentMessages.forEach((message, index) => {
    const isUser = message.role === 'user';
    const roleSymbol = isUser ? '>' : '⏺';
    const roleColor = isUser ? chalk.blue : chalk.cyan;
    
    // Add separator between messages for better readability
    if (index > 0) {
      console.log(chalk.gray('┈'.repeat(40)));
    }
    
    // Format message content
    let content = '';
    if (typeof message.content === 'string') {
      content = message.content;
    } else if (Array.isArray(message.content)) {
      content = message.content.map(item => item.text || '').join(' ');
    }
    
    // Handle special content types
    let displayContent = content;
    
    if (content.startsWith('🔧 Used tool:')) {
      const toolName = content.replace('🔧 Used tool: ', '');
      displayContent = chalk.yellow(`[Tool: ${toolName}]`);
    } else {
      // For extended preview, show more content
      const terminalWidth = process.stdout.columns || 80;
      const maxContentWidth = terminalWidth - 15; // Account for role label and spacing
      
      // Show more lines and characters for extended preview
      const lines = content.split('\n');
      const maxLines = 8; // Show up to 8 lines per message
      
      if (lines.length > maxLines) {
        const shownLines = lines.slice(0, maxLines);
        const lastLine = shownLines[shownLines.length - 1];
        
        // Truncate last shown line if needed
        if (lastLine && lastLine.length > maxContentWidth) {
          shownLines[shownLines.length - 1] = lastLine.substring(0, maxContentWidth - 3) + '...';
        }
        
        displayContent = shownLines.join('\n') + '\n' + 
          chalk.gray(`... (${lines.length - maxLines} more lines, ${content.length - shownLines.join('\n').length} more chars)`);
      } else {
        // Handle long single lines
        displayContent = lines.map(line => {
          if (line.length > maxContentWidth) {
            return line.substring(0, maxContentWidth - 3) + '...';
          }
          return line;
        }).join('\n');
      }
    }
    
    // Format with role
    const roleDisplay = roleColor(roleSymbol);
    
    // Handle multi-line display with Claude Code formatting
    const contentLines = displayContent.split('\n');
    contentLines.forEach((line, lineIndex) => {
      if (lineIndex === 0) {
        console.log(`${roleDisplay} ${line}`);
      } else {
        // Indent continuation lines
        console.log(`   ${line}`); // Simple indent for continuation lines
      }
    });
    
    // Add spacing between messages like Claude Code
    console.log();
  });
  
  if (messages.length > messagesToShow) {
    console.log();
    console.log(chalk.gray(`... and ${messages.length - messagesToShow} more messages above (${messages.length} total)`));
  }
}

/**
 * Conversation category for grouping
 */
interface ConversationCategory {
  type: 'recent' | 'this-week' | 'older';
  title: string;
  emoji: string;
}

/**
 * Categorized conversation with metadata
 */
interface CategorizedConversation {
  conversation: import('../claude-history.js').ClaudeConversation;
  category: ConversationCategory;
  index: number;
}

/**
 * Categorize conversations by activity recency
 */
function categorizeConversationsByActivity(
  conversations: import('../claude-history.js').ClaudeConversation[]
): CategorizedConversation[] {
  const now = Date.now();
  const oneHour = 60 * 60 * 1000;
  const oneDay = 24 * oneHour;
  const oneWeek = 7 * oneDay;

  return conversations.map((conversation, index) => {
    const age = now - conversation.lastActivity;
    
    let category: ConversationCategory;
    if (age < oneHour) {
      category = {
        type: 'recent',
        title: '🔥 Very Recent (within 1 hour)',
        emoji: '🔥'
      };
    } else if (age < oneDay) {
      category = {
        type: 'recent',
        title: '⚡ Recent (within 24 hours)',
        emoji: '⚡'
      };
    } else if (age < oneWeek) {
      category = {
        type: 'this-week', 
        title: '📅 This week',
        emoji: '📅'
      };
    } else {
      category = {
        type: 'older',
        title: '📚 Older conversations',
        emoji: '📚'
      };
    }

    return {
      conversation,
      category,
      index
    };
  }).sort((a, b) => {
    // First sort by category priority (recent -> this-week -> older)
    const categoryOrder = { 'recent': 0, 'this-week': 1, 'older': 2 };
    const categoryDiff = categoryOrder[a.category.type] - categoryOrder[b.category.type];
    if (categoryDiff !== 0) return categoryDiff;
    
    // Within each category, sort by most recent first
    return b.conversation.lastActivity - a.conversation.lastActivity;
  });
}

/**
 * Create conversation choices with grouping
 */
function createConversationChoices(
  categorizedConversations: CategorizedConversation[]
): Array<{ name: string; value: string; description?: string; disabled?: boolean }> {
  const choices: Array<{ name: string; value: string; description?: string; disabled?: boolean }> = [];
  
  // Group conversations by category
  const groups = new Map<string, CategorizedConversation[]>();
  groups.set('recent', []);
  groups.set('this-week', []);
  groups.set('older', []);
  
  for (const item of categorizedConversations) {
    const group = groups.get(item.category.type) || [];
    group.push(item);
    groups.set(item.category.type, group);
  }

  // Add groups in order
  const groupOrder = ['recent', 'this-week', 'older'] as const;
  
  for (const groupType of groupOrder) {
    const group = groups.get(groupType as string) || [];
    
    if (group.length === 0) continue;
    
    // Add group header
    const category = group[0]?.category;
    if (!category) continue;
    
    choices.push({
      name: `\n${category.title}`,
      value: `__header_${groupType}__`,
      disabled: true
    });
    
    // Add conversations in this group
    for (const { conversation, index } of group) {
      const formatted = formatConversationDisplay(conversation, index);
      choices.push(formatted);
    }
  }
  
  // Add separator before cancel option
  if (choices.length > 0) {
    choices.push({
      name: '',
      value: '__separator__',
      disabled: true
    });
  }
  
  return choices;
}

/**
 * Format conversation display
 */
function formatConversationDisplay(
  conversation: import('../claude-history.js').ClaudeConversation,
  index: number
): { name: string; value: string; description?: string } {
  const timeAgo = formatTimeAgo(conversation.lastActivity);
  const messageCount = conversation.messageCount;
  
  // Icon based on conversation content/title
  let icon = '💬';
  const lowerTitle = conversation.title.toLowerCase();
  if (lowerTitle.includes('bug') || lowerTitle.includes('fix') || lowerTitle.includes('error')) {
    icon = '🐛';
  } else if (lowerTitle.includes('feature') || lowerTitle.includes('implement') || lowerTitle.includes('add')) {
    icon = '🚀';
  } else if (lowerTitle.includes('doc') || lowerTitle.includes('readme') || lowerTitle.includes('comment')) {
    icon = '📝';
  } else if (lowerTitle.includes('test') || lowerTitle.includes('spec')) {
    icon = '🧪';
  }
  
  // Format: "  💬 Conversation title (X messages, time ago)"
  const title = conversation.title.length > 40 ? 
    conversation.title.substring(0, 37) + '...' : 
    conversation.title;
  
  const metadata = `(${messageCount} message${messageCount !== 1 ? 's' : ''}, ${chalk.gray(timeAgo)})`;
  
  // Create main display line
  const display = `  ${icon} ${chalk.cyan(title)} ${metadata}`;
  
  // Enhanced description with summary if available
  let description = '';
  if (conversation.summary && conversation.summary.trim()) {
    description = conversation.summary.length > 80 ? 
      conversation.summary.substring(0, 77) + '...' : 
      conversation.summary;
  } else {
    // Fallback description based on title analysis
    if (lowerTitle.includes('bug') || lowerTitle.includes('fix')) {
      description = 'Bug fix or error resolution';
    } else if (lowerTitle.includes('feature') || lowerTitle.includes('implement')) {
      description = 'Feature development or implementation';
    } else if (lowerTitle.includes('doc') || lowerTitle.includes('readme')) {
      description = 'Documentation or README updates';
    } else if (lowerTitle.includes('test')) {
      description = 'Testing and test improvements';
    } else {
      description = `${messageCount} messages exchanged ${timeAgo}`;
    }
  }
  
  return {
    name: display,
    value: index.toString(),
    description: description
  };
}

export async function selectClaudeExecutionMode(): Promise<{
  mode: 'normal' | 'continue' | 'resume';
  skipPermissions: boolean;
} | null> {
  try {
    const mode = await select({
      message: 'Select Claude Code execution mode (q to go back):',
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
  } catch {
    // Handle Ctrl+C or q key - user wants to go back
    return null;
  }
}

function formatTimeAgo(timestamp: number): string {
  const now = Date.now();
  const diff = now - timestamp;
  
  const minutes = Math.floor(diff / (1000 * 60));
  const hours = Math.floor(diff / (1000 * 60 * 60));
  const days = Math.floor(diff / (1000 * 60 * 60 * 24));
  const weeks = Math.floor(days / 7);
  const months = Math.floor(days / 30);
  
  if (minutes < 1) {
    return 'just now';
  } else if (minutes < 60) {
    return `${minutes}m ago`;
  } else if (hours < 24) {
    return `${hours}h ago`;
  } else if (days === 1) {
    return '1 day ago';
  } else if (days < 7) {
    return `${days} days ago`;
  } else if (weeks === 1) {
    return '1 week ago';
  } else if (weeks < 4) {
    return `${weeks} weeks ago`;
  } else if (months === 1) {
    return '1 month ago';
  } else {
    return `${months} months ago`;
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
 * Session status categories for grouping
 */
interface SessionCategory {
  type: 'active' | 'ready' | 'needs-attention';
  title: string;
  emoji: string;
  description: string;
}

/**
 * Enhanced session with category information
 */
interface CategorizedSession {
  session: SessionData;
  sessionInfo: import('../git.js').EnhancedSessionInfo;
  category: SessionCategory;
  index: number;
}

/**
 * Determine session category based on git status
 */
function categorizeSession(sessionInfo: import('../git.js').EnhancedSessionInfo): SessionCategory {
  if (sessionInfo.hasUncommittedChanges) {
    return {
      type: 'active',
      title: '🔥 Active (uncommitted changes)',
      emoji: '🔥',
      description: 'Sessions with ongoing work'
    };
  }
  
  if (sessionInfo.hasUnpushedCommits) {
    return {
      type: 'needs-attention', 
      title: '⚠️ Needs attention',
      emoji: '⚠️',
      description: 'Sessions with unpushed commits'
    };
  }
  
  return {
    type: 'ready',
    title: '✅ Ready to continue', 
    emoji: '✅',
    description: 'Clean sessions ready to resume'
  };
}

/**
 * Format session in new compact style
 */
function formatCompactSessionDisplay(
  session: SessionData, 
  sessionInfo: import('../git.js').EnhancedSessionInfo,
  index: number
): { name: string; value: string; description?: string } {
  const repo = session.repositoryRoot.split('/').pop() || 'unknown';
  const timeAgo = formatTimeAgo(session.timestamp);
  const branch = session.lastBranch || 'unknown';
  
  const projectIcon = getProjectIcon(repo);
  
  // Create status info in parentheses
  let statusInfo = '';
  if (sessionInfo.hasUncommittedChanges) {
    const count = sessionInfo.uncommittedChangesCount;
    statusInfo = `📝 ${count} file${count !== 1 ? 's' : ''}`;
  } else if (sessionInfo.hasUnpushedCommits) {
    const count = sessionInfo.unpushedCommitsCount;
    statusInfo = `🔄 ${count} commit${count !== 1 ? 's' : ''}`;
  }
  
  // Format: "  🚀 project-name → branch-name     (status, time)"
  const projectBranch = `${chalk.cyan(repo)} → ${chalk.green(branch)}`;
  const padding = Math.max(0, 35 - repo.length - branch.length);
  const timeAndStatus = statusInfo ? 
    `(${statusInfo}, ${chalk.gray(timeAgo)})` : 
    `(${chalk.gray(timeAgo)})`;
  
  const display = `  ${projectIcon} ${projectBranch}${' '.repeat(padding)} ${timeAndStatus}`;
  
  return {
    name: display,
    value: index.toString(),
    description: session.lastWorktreePath || ''
  };
}

/**
 * Group and sort sessions by category and priority
 */
function groupAndSortSessions(categorizedSessions: CategorizedSession[]): Map<string, CategorizedSession[]> {
  const groups = new Map<string, CategorizedSession[]>();
  
  // Initialize groups
  groups.set('active', []);
  groups.set('ready', []);
  groups.set('needs-attention', []);
  
  // Group sessions by category
  for (const session of categorizedSessions) {
    const group = groups.get(session.category.type) || [];
    group.push(session);
    groups.set(session.category.type, group);
  }
  
  // Sort within each group by timestamp (most recent first)
  for (const [key, sessions] of groups.entries()) {
    sessions.sort((a, b) => b.session.timestamp - a.session.timestamp);
    groups.set(key, sessions);
  }
  
  return groups;
}

/**
 * Create grouped choices for the prompt
 */
function createGroupedChoices(groupedSessions: Map<string, CategorizedSession[]>): Array<{ name: string; value: string; description?: string; disabled?: boolean }> {
  const choices: Array<{ name: string; value: string; description?: string; disabled?: boolean }> = [];
  
  // Define group order for display
  const groupOrder = ['active', 'needs-attention', 'ready'] as const;
  
  for (const groupType of groupOrder) {
    const sessions = groupedSessions.get(groupType) || [];
    
    if (sessions.length === 0) continue;
    
    // Add group header
    const category = sessions[0]?.category;
    if (!category) continue;
    
    choices.push({
      name: `
${category.title}`,
      value: `__header_${groupType}__`,
      disabled: true
    });
    
    // Add sessions in this group
    for (const { session, sessionInfo, index } of sessions) {
      const formatted = formatCompactSessionDisplay(session, sessionInfo, index);
      choices.push(formatted);
    }
  }
  
  // Add a separator before cancel option
  if (choices.length > 0) {
    choices.push({
      name: '',
      value: '__separator__',
      disabled: true
    });
  }
  
  return choices;
}


