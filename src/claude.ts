import { execa } from 'execa';
import chalk from 'chalk';
export class ClaudeError extends Error {
  constructor(message: string, public cause?: unknown) {
    super(message);
    this.name = 'ClaudeError';
  }
}

export async function launchClaudeCode(worktreePath: string, skipPermissions = false): Promise<void> {
  try {
    console.log(chalk.blue('🚀 Launching Claude Code...'));
    console.log(chalk.gray(`   Working directory: ${worktreePath}`));
    
    const args: string[] = [];
    if (skipPermissions) {
      args.push('--dangerously-skip-permissions');
      console.log(chalk.yellow('   ⚠️  Skipping permissions check'));
    }
    
    await execa('claude', args, {
      cwd: worktreePath,
      stdio: 'inherit'
    });
  } catch (error) {
    throw new ClaudeError('Failed to launch Claude Code', error);
  }
}

export async function isClaudeCodeAvailable(): Promise<boolean> {
  try {
    await execa('claude', ['--version']);
    return true;
  } catch {
    return false;
  }
}