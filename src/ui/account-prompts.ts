import { select, input, confirm } from '@inquirer/prompts';
import chalk from 'chalk';
import { claudeAccountService } from '../services/claude-account.service.js';

/**
 * 現在のアカウント情報を表示
 */
export async function showCurrentAccountInfo(): Promise<void> {
  console.log(chalk.blue('\n🔐 Current Claude Account Information'));
  console.log(chalk.gray('═'.repeat(50)));

  try {
    const authStatus = await claudeAccountService.checkAuthStatus();
    
    if (!authStatus.isAuthenticated) {
      console.log(chalk.red('❌ No active Claude authentication found'));
      console.log(chalk.yellow('   Run "claude setup-token" to authenticate'));
      return;
    }

    const { accountInfo, activeAccount } = authStatus;
    
    console.log(chalk.green('✅ Authenticated'));
    
    if (activeAccount) {
      console.log(chalk.cyan(`📛 Account Name: ${activeAccount}`));
    }
    
    if (accountInfo) {
      console.log(chalk.cyan(`📋 Plan: ${accountInfo.subscriptionType}`));
      console.log(chalk.cyan(`🔑 Scopes: ${accountInfo.scopes.join(', ')}`));
      
      const expirationDate = new Date(accountInfo.expiresAt);
      const isExpired = accountInfo.isExpired;
      
      console.log(chalk.cyan(`⏰ Expires: ${expirationDate.toLocaleString()}`));
      console.log(isExpired ? 
        chalk.red('❌ Token is expired') : 
        chalk.green('✅ Token is valid')
      );
    }
  } catch (error) {
    console.error(chalk.red('Failed to get account info:'), error);
  }
  
  console.log(); // 空行
}

/**
 * アカウント切り替えプロンプト  
 */
export async function selectAccountToSwitch(): Promise<string | null> {
  try {
    const accounts = await claudeAccountService.getSavedAccounts();
    
    if (accounts.length === 0) {
      console.log(chalk.yellow('⚠️  No saved accounts found'));
      console.log(chalk.gray('   Use "Save Current Account" to save an account first'));
      return null;
    }

    const currentActive = await claudeAccountService.getActiveAccountName();
    
    const choices = accounts.map(account => {
      const isActive = account.name === currentActive;
      const isExpired = account.expiresAt < Date.now();
      
      let name = account.name;
      let description = `${account.subscriptionType}`;
      
      if (isActive) {
        name = `${name} ${chalk.green('(current)')}`;
      }
      
      if (isExpired) {
        description += ` ${chalk.red('(expired)')}`;
      } else {
        description += ` ${chalk.green('(valid)')}`;
      }
      
      const savedDate = new Date(account.savedAt).toLocaleDateString();
      description += ` - saved ${savedDate}`;
      
      return {
        name,
        value: account.name,
        description
      };
    });

    choices.push({
      name: chalk.gray('← Back to menu'),
      value: '__back__',
      description: 'Return to account management menu'
    });

    const selected = await select({
      message: 'Select account to switch to:',
      choices
    });

    return selected === '__back__' ? null : selected;
  } catch (error) {
    console.error(chalk.red('Failed to load accounts:'), error);
    return null;
  }
}

/**
 * 現在のアカウントを保存するプロンプト
 */
export async function saveCurrentAccountPrompt(): Promise<string | null> {
  try {
    // 認証状況を確認
    const authStatus = await claudeAccountService.checkAuthStatus();
    
    if (!authStatus.isAuthenticated) {
      console.log(chalk.red('❌ No active Claude authentication found'));
      console.log(chalk.yellow('   Run "claude setup-token" to authenticate first'));
      return null;
    }

    console.log(chalk.blue('\n💾 Save Current Account'));
    console.log(chalk.gray('Current account details:'));
    
    if (authStatus.accountInfo) {
      console.log(chalk.cyan(`   Plan: ${authStatus.accountInfo.subscriptionType}`));
      console.log(chalk.cyan(`   Scopes: ${authStatus.accountInfo.scopes.join(', ')}`));
    }

    const accountName = await input({
      message: 'Enter a name for this account:',
      validate: (input: string) => {
        if (!input.trim()) {
          return 'Account name cannot be empty';
        }
        if (input.trim().length > 50) {
          return 'Account name must be 50 characters or less';
        }
        // 無効な文字をチェック
        if (!/^[a-zA-Z0-9\s\-_]+$/.test(input.trim())) {
          return 'Account name can only contain letters, numbers, spaces, hyphens, and underscores';
        }
        return true;
      }
    });

    return accountName.trim();
  } catch (error) {
    console.error(chalk.red('Error in save account prompt:'), error);
    return null;
  }
}

/**
 * アカウント削除のプロンプト
 */
export async function selectAccountToDelete(): Promise<string | null> {
  try {
    const accounts = await claudeAccountService.getSavedAccounts();
    
    if (accounts.length === 0) {
      console.log(chalk.yellow('⚠️  No saved accounts found'));
      return null;
    }

    const currentActive = await claudeAccountService.getActiveAccountName();
    
    const choices = accounts.map(account => {
      const isActive = account.name === currentActive;
      let name = account.name;
      let description = `${account.subscriptionType}`;
      
      if (isActive) {
        name = `${name} ${chalk.yellow('(currently active)')}`;
      }
      
      const savedDate = new Date(account.savedAt).toLocaleDateString();
      description += ` - saved ${savedDate}`;
      
      return {
        name,
        value: account.name,
        description
      };
    });

    choices.push({
      name: chalk.gray('← Back to menu'),
      value: '__back__',
      description: 'Return to account management menu'
    });

    const selected = await select({
      message: chalk.red('⚠️  Select account to DELETE:'),
      choices
    });

    if (selected === '__back__') {
      return null;
    }

    // 削除確認
    const confirmed = await confirm({
      message: `Are you sure you want to delete account "${selected}"?`,
      default: false
    });

    return confirmed ? selected : null;
  } catch (error) {
    console.error(chalk.red('Failed to delete account:'), error);
    return null;
  }
}

/**
 * 新しいアカウント追加のプロンプト
 */
export async function addNewAccountPrompt(): Promise<string | null> {
  try {
    console.log(chalk.blue('\n➕ Add New Claude Account'));
    console.log(chalk.gray('This will open your browser to authenticate with Claude'));
    console.log(chalk.yellow('⚠️  Make sure you\'re logged out of Claude or use incognito mode'));
    console.log(chalk.yellow('   to authenticate with a different account'));

    const shouldContinue = await confirm({
      message: 'Continue with authentication?',
      default: true
    });

    if (!shouldContinue) {
      return null;
    }

    const accountName = await input({
      message: 'Enter a name for the new account:',
      validate: (input: string) => {
        if (!input.trim()) {
          return 'Account name cannot be empty';
        }
        if (input.trim().length > 50) {
          return 'Account name must be 50 characters or less';
        }
        if (!/^[a-zA-Z0-9\s\-_]+$/.test(input.trim())) {
          return 'Account name can only contain letters, numbers, spaces, hyphens, and underscores';
        }
        return true;
      }
    });

    return accountName.trim();
  } catch (error) {
    console.error(chalk.red('Error in add account prompt:'), error);
    return null;
  }
}

/**
 * アカウント管理メインメニュー
 */
export async function showAccountManagementMenu(): Promise<string> {
  const choices = [
    {
      name: '📋 Show Current Account Info',
      value: 'show_info',
      description: 'Display current Claude account information'
    },
    {
      name: '🔄 Switch Account', 
      value: 'switch',
      description: 'Switch to a different saved account'
    },
    {
      name: '💾 Save Current Account',
      value: 'save',
      description: 'Save the current account with a custom name'
    },
    {
      name: '➕ Add New Account',
      value: 'add',
      description: 'Authenticate and add a new Claude account'
    },
    {
      name: '🗑️ Delete Account',
      value: 'delete',
      description: 'Remove a saved account'
    },
    {
      name: chalk.gray('← Back to Main Menu'),
      value: 'back',
      description: 'Return to main menu'
    }
  ];

  return await select({
    message: chalk.blue('🔐 Claude Account Management'),
    choices
  });
}

/**
 * アカウント管理のメインフロー
 */
export async function handleAccountManagement(): Promise<void> {
  while (true) {
    try {
      const action = await showAccountManagementMenu();

      switch (action) {
        case 'show_info':
          await showCurrentAccountInfo();
          break;

        case 'switch': {
          const accountToSwitch = await selectAccountToSwitch();
          if (accountToSwitch) {
            await claudeAccountService.switchToAccount(accountToSwitch);
            await showCurrentAccountInfo();
          }
          break;
        }

        case 'save': {
          const accountName = await saveCurrentAccountPrompt();
          if (accountName) {
            await claudeAccountService.saveCurrentAccount(accountName);
            console.log(chalk.green(`✅ Account "${accountName}" saved successfully!`));
          }
          break;
        }

        case 'add': {
          const newAccountName = await addNewAccountPrompt();
          if (newAccountName) {
            await claudeAccountService.addNewAccount(newAccountName);
          }
          break;
        }

        case 'delete': {
          const accountToDelete = await selectAccountToDelete();
          if (accountToDelete) {
            await claudeAccountService.deleteAccount(accountToDelete);
          }
          break;
        }

        case 'back':
          return;

        default:
          console.log(chalk.red('Unknown action'));
      }

      // 操作完了後、少し間を置く
      if (action !== 'back') {
        console.log(chalk.gray('\nPress Enter to continue...'));
        await new Promise(resolve => {
          process.stdin.once('data', () => resolve(undefined));
        });
      }

    } catch (error) {
      console.error(chalk.red('Error in account management:'), error);
      break;
    }
  }
}