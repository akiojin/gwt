import { vi } from 'vitest';
import type { ExecaReturnValue } from 'execa';

/**
 * execaのモック - Gitコマンド、GitHub CLI、AIツールの実行をモック
 */
export function mockExeca(defaultStdout: string = '', defaultStderr: string = '') {
  return vi.fn(async (command: string, args?: readonly string[]): Promise<Partial<ExecaReturnValue>> => {
    const fullCommand = args ? `${command} ${args.join(' ')}` : command;

    // Gitコマンドのモック
    if (command === 'git') {
      if (args?.[0] === 'branch') {
        if (args.includes('--list')) {
          return {
            stdout: '* main\n  feature/test\n  hotfix/bug',
            stderr: '',
            exitCode: 0,
          } as Partial<ExecaReturnValue>;
        }
        if (args.includes('-r')) {
          return {
            stdout: '  origin/main\n  origin/develop',
            stderr: '',
            exitCode: 0,
          } as Partial<ExecaReturnValue>;
        }
      }
      if (args?.[0] === 'worktree' && args[1] === 'list') {
        return {
          stdout: '/path/to/main  abc123 [main]\n/path/to/feature  def456 [feature/test]',
          stderr: '',
          exitCode: 0,
        } as Partial<ExecaReturnValue>;
      }
      if (args?.[0] === 'status') {
        return {
          stdout: 'On branch main\nnothing to commit, working tree clean',
          stderr: '',
          exitCode: 0,
        } as Partial<ExecaReturnValue>;
      }
    }

    // GitHub CLIのモック
    if (command === 'gh') {
      if (args?.[0] === 'pr' && args[1] === 'list') {
        return {
          stdout: JSON.stringify([
            {
              number: 123,
              title: 'Test PR',
              headRefName: 'feature/test',
              url: 'https://github.com/test/repo/pull/123',
              state: 'MERGED',
              mergedAt: '2025-01-01T00:00:00Z',
            },
          ]),
          stderr: '',
          exitCode: 0,
        } as Partial<ExecaReturnValue>;
      }
      if (args?.[0] === 'auth' && args[1] === 'status') {
        return {
          stdout: 'Logged in to github.com as testuser',
          stderr: '',
          exitCode: 0,
        } as Partial<ExecaReturnValue>;
      }
    }

    // Claude CodeとCodex CLIのモック
    if (command === 'claude' || command === 'codex') {
      return {
        stdout: `${command} executed successfully`,
        stderr: '',
        exitCode: 0,
      } as Partial<ExecaReturnValue>;
    }

    // デフォルトの応答
    return {
      stdout: defaultStdout,
      stderr: defaultStderr,
      exitCode: 0,
    } as Partial<ExecaReturnValue>;
  });
}

/**
 * inquirerのプロンプトモック
 */
export function mockInquirerPrompts<T = any>(responses: Record<string, T>) {
  return {
    select: vi.fn(async (config: any) => {
      const key = config.message || config.name;
      return responses[key] || config.choices?.[0]?.value;
    }),
    input: vi.fn(async (config: any) => {
      const key = config.message || config.name;
      return responses[key] || '';
    }),
    confirm: vi.fn(async (config: any) => {
      const key = config.message || config.name;
      return responses[key] !== undefined ? responses[key] : true;
    }),
    checkbox: vi.fn(async (config: any) => {
      const key = config.message || config.name;
      return responses[key] || [];
    }),
  };
}

/**
 * ファイルシステム操作のモック
 */
export function mockFileSystem() {
  return {
    readFile: vi.fn(async (path: string) => {
      if (path.includes('package.json')) {
        return JSON.stringify({ version: '1.0.0' });
      }
      if (path.includes('session.json')) {
        return JSON.stringify({
          lastWorktreePath: '/path/to/worktree',
          lastBranch: 'feature/test',
          timestamp: Date.now(),
          repositoryRoot: '/path/to/repo',
        });
      }
      return '';
    }),
    writeFile: vi.fn(async () => undefined),
    mkdir: vi.fn(async () => undefined),
    rm: vi.fn(async () => undefined),
    access: vi.fn(async () => undefined),
  };
}

/**
 * テスト用のBranchInfo生成ヘルパー
 */
export function createMockBranchInfo(overrides?: Partial<{
  name: string;
  type: 'local' | 'remote';
  branchType: 'feature' | 'hotfix' | 'release' | 'main' | 'develop' | 'other';
  isCurrent: boolean;
}>) {
  return {
    name: 'feature/test',
    type: 'local' as const,
    branchType: 'feature' as const,
    isCurrent: false,
    ...overrides,
  };
}

/**
 * テスト用のWorktreeInfo生成ヘルパー
 */
export function createMockWorktreeInfo(overrides?: Partial<{
  branch: string;
  path: string;
  isAccessible: boolean;
}>) {
  return {
    branch: 'feature/test',
    path: '/path/to/worktree',
    isAccessible: true,
    ...overrides,
  };
}

/**
 * テスト用のSessionData生成ヘルパー
 */
export function createMockSessionData(overrides?: Partial<{
  lastWorktreePath: string | null;
  lastBranch: string | null;
  timestamp: number;
  repositoryRoot: string;
}>) {
  return {
    lastWorktreePath: '/path/to/worktree',
    lastBranch: 'feature/test',
    timestamp: Date.now(),
    repositoryRoot: '/path/to/repo',
    ...overrides,
  };
}

/**
 * テスト用のCleanupTarget生成ヘルパー
 */
export function createMockCleanupTarget(overrides?: Partial<{
  branch: string;
  worktreePath: string | undefined;
  prNumber: number;
  prUrl: string;
  hasUnpushedCommits: boolean;
  hasRemoteBranch: boolean;
  cleanupType: 'worktree-and-branch' | 'branch-only';
}>) {
  return {
    branch: 'feature/test',
    worktreePath: '/path/to/worktree',
    prNumber: 123,
    prUrl: 'https://github.com/test/repo/pull/123',
    hasUnpushedCommits: false,
    hasRemoteBranch: true,
    cleanupType: 'worktree-and-branch' as const,
    ...overrides,
  };
}

/**
 * コンソール出力のモック（表示確認用）
 */
export function mockConsole() {
  return {
    log: vi.fn(),
    error: vi.fn(),
    warn: vi.fn(),
    info: vi.fn(),
  };
}

/**
 * プロセス終了のモック
 */
export function mockProcess() {
  return {
    exit: vi.fn(),
    cwd: vi.fn(() => '/path/to/repo'),
  };
}
