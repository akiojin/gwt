import { execa } from 'execa';
import chalk from 'chalk';
import { platform } from 'os';
import { existsSync } from 'fs';

const CODEX_CLI_PACKAGE = '@openai/codex';

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
      args.push('--yolo');
      console.log(chalk.yellow('   ⚠️  Bypassing approvals and sandbox'));
    }

    if (options.extraArgs && options.extraArgs.length > 0) {
      args.push(...options.extraArgs);
    }

    args.push('--search');

    await execa('npx', ['--yes', CODEX_CLI_PACKAGE, ...args], {
      cwd: worktreePath,
      stdio: 'inherit',
      shell: true
    });
  } catch (error: any) {
    const errorMessage = error.code === 'ENOENT'
      ? 'npx command not found. Please ensure Node.js/npm is installed so Codex CLI can run via npx.'
      : `Failed to launch Codex CLI: ${error.message || 'Unknown error'}`;

    if (platform() === 'win32') {
      console.error(chalk.red('\n💡 Windows troubleshooting tips:'));
      console.error(chalk.yellow('   1. Ensure Node.js/npm がインストールされ npx が利用可能か確認'));
      console.error(chalk.yellow('   2. "npx @openai/codex -- --help" を実行してセットアップを確認'));
      console.error(chalk.yellow('   3. ターミナルやIDEを再起動して PATH を更新'));
    }

    throw new CodexError(errorMessage, error);
  }
}

export async function isCodexAvailable(): Promise<boolean> {
  try {
    await execa('npx', ['--yes', CODEX_CLI_PACKAGE, '--help'], { shell: true });
    return true;
  } catch (error: any) {
    if (error.code === 'ENOENT') {
      console.error(chalk.yellow('\n⚠️  npx コマンドが見つかりません'));
    }
    return false;
  }
}
