import { homedir } from 'node:os';
import path from 'node:path';
import { readFile } from 'node:fs/promises';

export interface AppConfig {
  defaultBaseBranch: string;
  skipPermissions: boolean;
  enableGitHubIntegration: boolean;
  enableDebugMode: boolean;
  worktreeNamingPattern: string;
}

const DEFAULT_CONFIG: AppConfig = {
  defaultBaseBranch: 'main',
  skipPermissions: false,
  enableGitHubIntegration: true,
  enableDebugMode: false,
  worktreeNamingPattern: '{repo}-{branch}'
};

/**
 * 設定ファイルを読み込む
 */
export async function loadConfig(): Promise<AppConfig> {
  const configPaths = [
    path.join(process.cwd(), '.claude-worktree.json'),
    path.join(homedir(), '.config', 'claude-worktree', 'config.json'),
    path.join(homedir(), '.claude-worktree.json')
  ];
  
  for (const configPath of configPaths) {
    try {
      const content = await readFile(configPath, 'utf-8');
      const userConfig = JSON.parse(content);
      return { ...DEFAULT_CONFIG, ...userConfig };
    } catch (error) {
      // 設定ファイルが見つからない場合は次を試す
      if (process.env.DEBUG_CONFIG) {
        console.error(`Failed to load config from ${configPath}:`, error instanceof Error ? error.message : String(error));
      }
    }
  }
  
  // 環境変数からも読み込み
  return {
    ...DEFAULT_CONFIG,
    defaultBaseBranch: process.env.CLAUDE_WORKTREE_BASE_BRANCH || DEFAULT_CONFIG.defaultBaseBranch,
    skipPermissions: process.env.CLAUDE_WORKTREE_SKIP_PERMISSIONS === 'true',
    enableGitHubIntegration: process.env.CLAUDE_WORKTREE_GITHUB !== 'false',
    enableDebugMode: process.env.DEBUG_CLEANUP === 'true' || process.env.DEBUG === 'true'
  };
}

/**
 * 設定値を取得する
 */
let cachedConfig: AppConfig | null = null;

export async function getConfig(): Promise<AppConfig> {
  if (!cachedConfig) {
    cachedConfig = await loadConfig();
  }
  return cachedConfig;
}

export function resetConfigCache(): void {
  cachedConfig = null;
}