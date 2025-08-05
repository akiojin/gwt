import { execa } from 'execa';
import chalk from 'chalk';

/**
 * Claude Config統合サービス
 * Claude Codeの設定システムを統合的に管理
 */
export class ClaudeConfigService {
  
  /**
   * 設定値を取得
   */
  async get(key: string, global = false): Promise<string | null> {
    try {
      const args = ['config', 'get'];
      if (global) args.push('-g');
      args.push(key);
      
      const { stdout } = await execa('claude', args);
      return stdout.trim() || null;
    } catch (error: any) {
      if (error.stderr?.includes('not found') || error.exitCode === 1) {
        return null;
      }
      throw error;
    }
  }

  /**
   * 設定値を設定
   */
  async set(key: string, value: string, global = false): Promise<void> {
    const args = ['config', 'set'];
    if (global) args.push('-g');
    args.push(key, value);
    
    await execa('claude', args);
  }

  /**
   * 設定値を削除
   */
  async remove(key: string, global = false): Promise<void> {
    const args = ['config', 'remove'];
    if (global) args.push('-g');
    args.push(key);
    
    await execa('claude', args);
  }

  /**
   * 全設定を取得
   */
  async list(global = false): Promise<Record<string, any>> {
    try {
      const args = ['config', 'list'];
      if (global) args.push('-g');
      
      const { stdout } = await execa('claude', args);
      return JSON.parse(stdout);
    } catch (error) {
      console.error(chalk.red('Failed to list Claude config:'), error);
      return {};
    }
  }

  /**
   * 配列形式の設定に値を追加
   */
  async add(key: string, values: string[], global = false): Promise<void> {
    const args = ['config', 'add'];
    if (global) args.push('-g');
    args.push(key, ...values);
    
    await execa('claude', args);
  }

  /**
   * アカウント管理用の設定キーを生成
   */
  getAccountConfigKey(accountName: string): string {
    return `claudeWorktree.accounts.${accountName}`;
  }

  /**
   * アクティブアカウント設定キー
   */
  getActiveAccountKey(): string {
    return 'claudeWorktree.activeAccount';
  }

  /**
   * 保存済みアカウント一覧を取得
   */
  async getSavedAccounts(): Promise<string[]> {
    try {
      const config = await this.list(true);
      const accounts: string[] = [];
      
      // claudeWorktree.accounts.* の形式の設定を探す
      for (const key in config) {
        if (key.startsWith('claudeWorktree.accounts.')) {
          const accountName = key.replace('claudeWorktree.accounts.', '');
          accounts.push(accountName);
        }
      }
      
      return accounts;
    } catch (error) {
      console.error(chalk.red('Failed to get saved accounts:'), error);
      return [];
    }
  }

  /**
   * アクティブなアカウント名を取得
   */
  async getActiveAccount(): Promise<string | null> {
    return await this.get(this.getActiveAccountKey(), true);
  }

  /**
   * アクティブなアカウントを設定
   */
  async setActiveAccount(accountName: string): Promise<void> {
    await this.set(this.getActiveAccountKey(), accountName, true);
  }

  /**
   * アカウント情報を保存
   */
  async saveAccountConfig(accountName: string, accountData: any): Promise<void> {
    const key = this.getAccountConfigKey(accountName);
    await this.set(key, JSON.stringify(accountData), true);
  }

  /**
   * アカウント情報を取得
   */
  async getAccountConfig(accountName: string): Promise<any | null> {
    const key = this.getAccountConfigKey(accountName);
    const data = await this.get(key, true);
    
    if (!data) return null;
    
    try {
      return JSON.parse(data);
    } catch (error) {
      console.error(chalk.red(`Failed to parse account config for ${accountName}:`), error);
      return null;
    }
  }

  /**
   * アカウント設定を削除
   */
  async removeAccountConfig(accountName: string): Promise<void> {
    const key = this.getAccountConfigKey(accountName);
    await this.remove(key, true);
  }
}

export const claudeConfigService = new ClaudeConfigService();