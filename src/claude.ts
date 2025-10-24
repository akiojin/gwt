import { execa } from 'execa';
import chalk from 'chalk';
import { platform } from 'os';
import { existsSync } from 'fs';

const CLAUDE_CLI_PACKAGE = '@anthropic-ai/claude-code';
export class ClaudeError extends Error {
  constructor(message: string, public cause?: unknown) {
    super(message);
    this.name = 'ClaudeError';
  }
}

export async function launchClaudeCode(
  worktreePath: string, 
  options: {
    skipPermissions?: boolean;
    mode?: 'normal' | 'continue' | 'resume';
    extraArgs?: string[];
  } = {}
): Promise<void> {
  try {
    // Check if the worktree path exists
    if (!existsSync(worktreePath)) {
      throw new Error(`Worktree path does not exist: ${worktreePath}`);
    }
    
    console.log(chalk.blue('🚀 Launching Claude Code...'));
    console.log(chalk.gray(`   Working directory: ${worktreePath}`));
    
    const args: string[] = [];
    
    // Handle execution mode
    switch (options.mode) {
      case 'continue':
        args.push('-c');
        console.log(chalk.cyan('   📱 Continuing most recent conversation'));
        break;
      case 'resume':
        // Use our custom conversation selection instead of claude -r
        console.log(chalk.cyan('   🔄 Selecting conversation to resume'));
        
        try {
          const { selectClaudeConversation } = await import('./ui/prompts.js');
          const selectedConversation = await selectClaudeConversation(worktreePath);
          
          if (selectedConversation) {
            console.log(chalk.green(`   ✨ Resuming: ${selectedConversation.title}`));
            
            // Use specific session ID if available
            if (selectedConversation.sessionId) {
              args.push('--resume', selectedConversation.sessionId);
              console.log(chalk.cyan(`   🆔 Using session ID: ${selectedConversation.sessionId}`));
            } else {
              // Fallback: try to use filename as session identifier
              const fileName = selectedConversation.id;
              console.log(chalk.yellow(`   ⚠️  No session ID found, trying filename: ${fileName}`));
              args.push('--resume', fileName);
            }
          } else {
            // User cancelled - return without launching Claude
            console.log(chalk.gray('   ↩️  Selection cancelled, returning to menu'));
            return;
          }
        } catch (error) {
          console.warn(chalk.yellow('   ⚠️  Failed to load conversation history, using standard resume'));
          args.push('-r');
        }
        break;
      case 'normal':
      default:
        console.log(chalk.green('   ✨ Starting new session'));
        break;
    }
    
    // Handle skip permissions
    if (options.skipPermissions) {
      args.push('--dangerously-skip-permissions');
      console.log(chalk.yellow('   ⚠️  Skipping permissions check'));
    }
    // Append any pass-through arguments after our flags
    if (options.extraArgs && options.extraArgs.length > 0) {
      args.push(...options.extraArgs);
    }

    await execa('npx', ['--yes', CLAUDE_CLI_PACKAGE, ...args], {
      cwd: worktreePath,
      stdio: 'inherit',
      shell: true
    });
  } catch (error: any) {
    const errorMessage = error.code === 'ENOENT' 
      ? 'npx command not found. Please ensure Node.js/npm is installed so Claude Code can run via npx.'
      : `Failed to launch Claude Code: ${error.message || 'Unknown error'}`;

    if (platform() === 'win32') {
      console.error(chalk.red('\n💡 Windows troubleshooting tips:'));
      console.error(chalk.yellow('   1. Ensure Node.js/npm がインストールされ npx が利用可能か確認'));
      console.error(chalk.yellow('   2. "npx @anthropic-ai/claude-code -- --version" を実行してセットアップを確認'));
      console.error(chalk.yellow('   3. ターミナルやIDEを再起動して PATH を更新'));
    }

    throw new ClaudeError(errorMessage, error);
  }
}

export async function isClaudeCodeAvailable(): Promise<boolean> {
  try {
    await execa('npx', ['--yes', CLAUDE_CLI_PACKAGE, '--version'], { shell: true });
    return true;
  } catch (error: any) {
    if (error.code === 'ENOENT') {
      console.error(chalk.yellow('\n⚠️  npx コマンドが見つかりません'));
      console.error(chalk.gray('   Node.js/npm をインストールして npx が使用可能か確認してください'));
    }
    return false;
  }
}
