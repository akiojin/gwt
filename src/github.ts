import { execa } from 'execa';
import chalk from 'chalk';
import type { PullRequest, MergedPullRequest } from './ui/types.js';

export async function isGitHubCLIAvailable(): Promise<boolean> {
  try {
    await execa('gh', ['--version']);
    return true;
  } catch {
    return false;
  }
}

export async function getPullRequests(): Promise<PullRequest[]> {
  try {
    // リモート情報を更新してから PR を取得
    try {
      await execa('git', ['fetch', '--all', '--prune']);
    } catch (fetchError) {
      if (process.env.DEBUG_CLEANUP) {
        console.log(chalk.yellow('Debug: Failed to fetch remote updates, continuing anyway'));
      }
    }
    
    const { stdout } = await execa('gh', [
      'pr', 'list',
      '--state', 'all',
      '--json', 'number,title,state,headRefName,mergedAt,author',
      '--limit', '100'
    ]);
    
    const prs = JSON.parse(stdout);
    return prs.map((pr: any) => ({
      number: pr.number,
      title: pr.title,
      state: pr.state,
      branch: pr.headRefName,
      mergedAt: pr.mergedAt,
      author: pr.author?.login || 'unknown'
    }));
  } catch (error) {
    console.error(chalk.yellow('Warning: Failed to fetch pull requests'));
    return [];
  }
}

export async function getMergedPullRequests(): Promise<MergedPullRequest[]> {
  try {
    // リモート情報を更新してから マージ済みPR を取得
    try {
      await execa('git', ['fetch', '--all', '--prune']);
    } catch (fetchError) {
      if (process.env.DEBUG_CLEANUP) {
        console.log(chalk.yellow('Debug: Failed to fetch remote updates, continuing anyway'));
      }
    }
    
    const { stdout } = await execa('gh', [
      'pr', 'list',
      '--state', 'merged',
      '--json', 'number,title,headRefName,mergedAt,author',
      '--limit', '100'
    ]);
    
    if (!stdout || stdout.trim() === '') {
      if (process.env.DEBUG_CLEANUP) {
        console.log(chalk.yellow('Debug: GitHub CLI returned empty response for merged PRs'));
      }
      return [];
    }
    
    const prs = JSON.parse(stdout);
    if (process.env.DEBUG_CLEANUP) {
      console.log(chalk.cyan(`Debug: GitHub CLI returned ${prs.length} merged PRs`));
    }
    
    return prs.map((pr: any) => ({
      number: pr.number,
      title: pr.title,
      branch: pr.headRefName,
      mergedAt: pr.mergedAt,
      author: pr.author?.login || 'unknown'
    }));
  } catch (error) {
    const errorMessage = error instanceof Error ? error.message : String(error);
    console.error(chalk.red('Error: Failed to fetch merged pull requests'));
    console.error(chalk.red(`Details: ${errorMessage}`));
    
    if (errorMessage.includes('authentication') || errorMessage.includes('auth')) {
      console.log(chalk.yellow('Hint: Try running "gh auth login" to authenticate with GitHub'));
    } else if (errorMessage.includes('not found') || errorMessage.includes('404')) {
      console.log(chalk.yellow('Hint: Make sure you are in a repository with GitHub remote'));
    }
    
    if (process.env.DEBUG_CLEANUP) {
      console.error(chalk.red('Debug: Full error details:'), error);
    }
    
    return [];
  }
}

export async function getPullRequestByBranch(branchName: string): Promise<PullRequest | null> {
  try {
    const { stdout } = await execa('gh', [
      'pr', 'list',
      '--head', branchName,
      '--state', 'all',
      '--json', 'number,title,state,headRefName,mergedAt,author',
      '--limit', '1'
    ]);
    
    const prs = JSON.parse(stdout);
    if (prs.length === 0) {
      return null;
    }
    
    const pr = prs[0];
    return {
      number: pr.number,
      title: pr.title,
      state: pr.state,
      branch: pr.headRefName,
      mergedAt: pr.mergedAt,
      author: pr.author?.login || 'unknown'
    };
  } catch (error) {
    return null;
  }
}

export async function checkGitHubAuth(): Promise<boolean> {
  try {
    // gh auth status returns non-zero exit code even when authenticated
    // Check if we can actually use the API instead
    await execa('gh', ['api', 'user']);
    return true;
  } catch {
    console.error(chalk.red('Error: GitHub CLI is not authenticated'));
    console.log(chalk.yellow('Please run: gh auth login'));
    return false;
  }
}