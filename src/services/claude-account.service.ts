import { readFile, writeFile, mkdir, copyFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import path from 'node:path';
import { homedir } from 'node:os';
import { execa } from 'execa';
import chalk from 'chalk';
import { claudeConfigService } from './claude-config.service.js';

interface ClaudeCredentials {
  claudeAiOauth: {
    accessToken: string;
    refreshToken: string;
    expiresAt: number;
    scopes: string[];
    subscriptionType: string;
  };
}

interface SavedAccount {
  name: string;
  subscriptionType: string;
  scopes: string[];
  expiresAt: number;
  savedAt: number;
  credentials: ClaudeCredentials;
}

/**
 * Claude Code アカウント管理サービス
 */
export class ClaudeAccountService {
  private readonly claudeCredentialsPath = path.join(homedir(), '.claude', '.credentials.json');
  private readonly accountsDir = path.join(homedir(), '.config', 'claude-worktree', 'accounts');

  constructor() {
    this.ensureAccountsDir();
  }

  /**
   * アカウント保存ディレクトリを作成
   */
  private async ensureAccountsDir(): Promise<void> {
    try {
      await mkdir(this.accountsDir, { recursive: true });
    } catch {
      // ディレクトリ作成エラーは無視（既に存在する場合）
    }
  }

  /**
   * 現在のClaude認証情報を読み取り
   */
  async getCurrentCredentials(): Promise<ClaudeCredentials | null> {
    try {
      if (!existsSync(this.claudeCredentialsPath)) {
        return null;
      }

      const content = await readFile(this.claudeCredentialsPath, 'utf-8');
      return JSON.parse(content) as ClaudeCredentials;
    } catch (error) {
      console.error(chalk.red('Failed to read Claude credentials:'), error);
      return null;
    }
  }

  /**
   * 現在のアカウント情報を取得
   */
  async getCurrentAccountInfo(): Promise<{
    subscriptionType: string;
    scopes: string[];
    expiresAt: number;
    isExpired: boolean;
  } | null> {
    const credentials = await this.getCurrentCredentials();
    if (!credentials?.claudeAiOauth) {
      return null;
    }

    const { subscriptionType, scopes, expiresAt } = credentials.claudeAiOauth;
    const isExpired = expiresAt < Date.now();

    return {
      subscriptionType,
      scopes,
      expiresAt,
      isExpired
    };
  }

  /**
   * 現在のアカウントを名前を付けて保存
   */
  async saveCurrentAccount(accountName: string): Promise<void> {
    const credentials = await this.getCurrentCredentials();
    if (!credentials) {
      throw new Error('No current Claude credentials found');
    }

    const accountInfo = await this.getCurrentAccountInfo();
    if (!accountInfo) {
      throw new Error('Failed to get current account info');
    }

    const savedAccount: SavedAccount = {
      name: accountName,
      subscriptionType: accountInfo.subscriptionType,
      scopes: accountInfo.scopes,
      expiresAt: accountInfo.expiresAt,
      savedAt: Date.now(),
      credentials
    };

    // ファイルに保存
    const accountPath = path.join(this.accountsDir, `${accountName}.json`);
    await writeFile(accountPath, JSON.stringify(savedAccount, null, 2), 'utf-8');

    // Claude Config にも保存
    await claudeConfigService.saveAccountConfig(accountName, {
      subscriptionType: accountInfo.subscriptionType,
      savedAt: Date.now()
    });

    console.log(chalk.green(`✅ Account "${accountName}" saved successfully`));
  }

  /**
   * 保存済みアカウント一覧を取得
   */
  async getSavedAccounts(): Promise<SavedAccount[]> {
    try {
      await this.ensureAccountsDir();
      const { readdir } = await import('node:fs/promises');
      
      const files = await readdir(this.accountsDir);
      const accounts: SavedAccount[] = [];

      for (const file of files) {
        if (!file.endsWith('.json')) continue;
        
        try {
          const filePath = path.join(this.accountsDir, file);
          const content = await readFile(filePath, 'utf-8');
          const account = JSON.parse(content) as SavedAccount;
          accounts.push(account);
        } catch (error) {
          console.error(chalk.yellow(`⚠️  Failed to load account file ${file}:`), error);
        }
      }

      // 保存日時順にソート（新しいものが先）
      accounts.sort((a, b) => b.savedAt - a.savedAt);

      return accounts;
    } catch (error) {
      console.error(chalk.red('Failed to get saved accounts:'), error);
      return [];
    }
  }

  /**
   * 指定したアカウントに切り替え
   */
  async switchToAccount(accountName: string): Promise<void> {
    const accounts = await this.getSavedAccounts();
    const targetAccount = accounts.find(acc => acc.name === accountName);

    if (!targetAccount) {
      throw new Error(`Account "${accountName}" not found`);
    }

    console.log(chalk.gray(`[DEBUG] Target account data:`));
    console.log(chalk.gray(`  Name: ${targetAccount.name}`));
    console.log(chalk.gray(`  SubscriptionType: ${targetAccount.subscriptionType}`));
    console.log(chalk.gray(`  Scopes: ${targetAccount.scopes.join(', ')}`));
    console.log(chalk.gray(`  ExpiresAt: ${new Date(targetAccount.expiresAt).toISOString()}`));

    // 現在の認証情報をバックアップ
    await this.backupCurrentCredentials();

    // 新しい認証情報を適用
    await writeFile(
      this.claudeCredentialsPath, 
      JSON.stringify(targetAccount.credentials, null, 2), 
      'utf-8'
    );

    // アクティブアカウントを記録
    await claudeConfigService.setActiveAccount(accountName);

    console.log(chalk.green(`🔄 Switched to account: ${accountName}`));
    console.log(chalk.cyan(`   Plan: ${targetAccount.subscriptionType || 'Unknown'}`));
    console.log(chalk.cyan(`   Scopes: ${targetAccount.scopes.join(', ')}`));

    // 切り替え後の認証情報を確認
    const newCredentials = await this.getCurrentCredentials();
    if (newCredentials) {
      console.log(chalk.gray(`[DEBUG] New credentials applied:`));
      console.log(chalk.gray(`  SubscriptionType: ${newCredentials.claudeAiOauth.subscriptionType}`));
      console.log(chalk.gray(`  Scopes: ${newCredentials.claudeAiOauth.scopes.join(', ')}`));
    }
  }

  /**
   * 現在の認証情報をバックアップ
   */
  private async backupCurrentCredentials(): Promise<void> {
    try {
      if (existsSync(this.claudeCredentialsPath)) {
        const backupPath = `${this.claudeCredentialsPath}.backup`;
        await copyFile(this.claudeCredentialsPath, backupPath);
      }
    } catch (error) {
      console.error(chalk.yellow('⚠️  Failed to backup credentials:'), error);
    }
  }

  /**
   * アカウントを削除
   */
  async deleteAccount(accountName: string): Promise<void> {
    const accountPath = path.join(this.accountsDir, `${accountName}.json`);
    
    try {
      const { unlink } = await import('node:fs/promises');
      await unlink(accountPath);
      
      // Claude Config からも削除
      await claudeConfigService.removeAccountConfig(accountName);
      
      // アクティブアカウントが削除対象の場合はクリア
      const activeAccount = await claudeConfigService.getActiveAccount();
      if (activeAccount === accountName) {
        await claudeConfigService.remove(claudeConfigService.getActiveAccountKey(), true);
      }

      console.log(chalk.green(`🗑️  Account "${accountName}" deleted successfully`));
    } catch (error) {
      console.error(chalk.red(`Failed to delete account "${accountName}":`), error);
      throw error;
    }
  }

  /**
   * 新しいアカウントを追加（setup-token を使用）
   */
  async addNewAccount(accountName: string): Promise<void> {
    console.log(chalk.blue('🔐 Setting up new Claude account...'));
    console.log(chalk.gray('   This will open your browser to authenticate with Claude'));

    try {
      // setup-token コマンドを実行
      await execa('claude', ['setup-token'], {
        stdio: 'inherit'
      });

      // 新しい認証情報を保存
      await this.saveCurrentAccount(accountName);
      
      console.log(chalk.green(`✨ New account "${accountName}" added successfully!`));
    } catch (error) {
      console.error(chalk.red('Failed to add new account:'), error);
      throw error;
    }
  }

  /**
   * アクティブなアカウント名を取得
   */
  async getActiveAccountName(): Promise<string | null> {
    return await claudeConfigService.getActiveAccount();
  }

  /**
   * 認証状況をチェック
   */
  async checkAuthStatus(): Promise<{
    isAuthenticated: boolean;
    activeAccount: string | null;
    accountInfo: any;
  }> {
    const credentials = await this.getCurrentCredentials();
    const activeAccount = await this.getActiveAccountName();
    const accountInfo = await this.getCurrentAccountInfo();

    return {
      isAuthenticated: !!credentials,
      activeAccount,
      accountInfo
    };
  }
}

export const claudeAccountService = new ClaudeAccountService();