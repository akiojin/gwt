import { execa } from 'execa';
import chalk from 'chalk';
import { platform } from 'os';
import { existsSync } from 'fs';
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
        args.push('-r');
        console.log(chalk.cyan('   🔄 Resuming selected conversation'));
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
    
    const isWindows = platform() === 'win32';
    
    await execa('claude', args, {
      cwd: worktreePath,
      stdio: 'inherit',
      shell: true
    });
  } catch (error: any) {
    const errorMessage = error.code === 'ENOENT' 
      ? 'Claude Code command not found. Please ensure Claude Code is installed and available in your PATH.'
      : `Failed to launch Claude Code: ${error.message || 'Unknown error'}`;
    
    if (platform() === 'win32') {
      console.error(chalk.red('\n💡 Windows troubleshooting tips:'));
      console.error(chalk.yellow('   1. Ensure Claude Code is installed: pnpm add -g @anthropic-ai/claude-code'));
      console.error(chalk.yellow('   2. Try restarting your terminal or IDE'));
      console.error(chalk.yellow('   3. Check if "claude" is available in your PATH'));
      console.error(chalk.yellow('   4. Try running "where claude" or "npx claude" to test the command'));
    }
    
    throw new ClaudeError(errorMessage, error);
  }
}

export async function isClaudeCodeAvailable(): Promise<boolean> {
  try {
    const isWindows = platform() === 'win32';
    await execa('claude', ['--version'], { shell: true });
    return true;
  } catch (error: any) {
    if (error.code === 'ENOENT') {
      console.error(chalk.yellow('\n⚠️  Claude Code not found in PATH'));
      console.error(chalk.gray('   Please install Claude Code: pnpm add -g @anthropic-ai/claude-code'));
    }
    return false;
  }
}