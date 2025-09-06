import { execa } from 'execa';
import chalk from 'chalk';
import { platform } from 'os';
import { existsSync } from 'fs';

export class CodexError extends Error {
  constructor(message: string, public cause?: unknown) {
    super(message);
    this.name = 'CodexError';
  }
}

export async function launchCodexCLI(
  worktreePath: string,
  options: {
    mode?: 'normal' | 'continue' | 'resume';
    extraArgs?: string[];
    bypassApprovals?: boolean;
  } = {}
): Promise<void> {
  try {
    if (!existsSync(worktreePath)) {
      throw new Error(`Worktree path does not exist: ${worktreePath}`);
    }

    console.log(chalk.blue('🚀 Launching Codex CLI...'));
    console.log(chalk.gray(`   Working directory: ${worktreePath}`));

    const args: string[] = [];

    switch (options.mode) {
      case 'continue':
        args.push('--continue');
        console.log(chalk.cyan('   ⏭️  Continue mode'));
        break;
      case 'resume':
        args.push('--resume');
        console.log(chalk.cyan('   🔄 Resume mode'));
        break;
      case 'normal':
      default:
        console.log(chalk.green('   ✨ Starting new session'));
        break;
    }

    if (options.bypassApprovals) {
      args.push('--dangerously-bypass-approvals-and-sandbox');
      console.log(chalk.yellow('   ⚠️  Bypassing approvals and sandbox'));
    }

    if (options.extraArgs && options.extraArgs.length > 0) {
      args.push(...options.extraArgs);
    }

    await execa('codex', args, {
      cwd: worktreePath,
      stdio: 'inherit',
      shell: true
    });
  } catch (error: any) {
    const errorMessage = error.code === 'ENOENT'
      ? 'Codex CLI command not found. Please ensure Codex CLI is installed and available in your PATH.'
      : `Failed to launch Codex CLI: ${error.message || 'Unknown error'}`;

    if (platform() === 'win32') {
      console.error(chalk.red('\n💡 Windows troubleshooting tips:'));
      console.error(chalk.yellow('   1. Ensure Codex CLI is installed globally'));
      console.error(chalk.yellow('   2. Restart your terminal or IDE'));
      console.error(chalk.yellow('   3. Check if "codex" is available in your PATH'));
    }

    throw new CodexError(errorMessage, error);
  }
}

export async function isCodexAvailable(): Promise<boolean> {
  try {
    await execa('codex', ['--help'], { shell: true });
    return true;
  } catch (error: any) {
    if (error.code === 'ENOENT') {
      console.error(chalk.yellow('\n⚠️  Codex CLI not found in PATH'));
    }
    return false;
  }
}
