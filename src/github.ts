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
    const { stdout } = await execa('gh', [
      'pr', 'list',
      '--state', 'merged',
      '--json', 'number,title,headRefName,mergedAt,author',
      '--limit', '100'
    ]);
    
    const prs = JSON.parse(stdout);
    return prs.map((pr: any) => ({
      number: pr.number,
      title: pr.title,
      branch: pr.headRefName,
      mergedAt: pr.mergedAt,
      author: pr.author?.login || 'unknown'
    }));
  } catch (error) {
    console.error(chalk.yellow('Warning: Failed to fetch merged pull requests'));
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
    await execa('gh', ['auth', 'status']);
    return true;
  } catch {
    console.error(chalk.red('Error: GitHub CLI is not authenticated'));
    console.log(chalk.yellow('Please run: gh auth login'));
    return false;
  }
}