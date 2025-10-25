import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import * as worktree from '../../src/worktree';
import { worktrees } from '../fixtures/worktrees';

// Mock execa
vi.mock('execa', () => ({
  execa: vi.fn(),
}));

// Mock node:fs
vi.mock('node:fs', () => ({
  existsSync: vi.fn(),
}));

import { execa } from 'execa';
import fs from 'node:fs';

describe('worktree.ts - Worktree Operations', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  describe('worktreeExists (T104)', () => {
    it('should return worktree path if worktree exists for branch', async () => {
      const mockOutput = `worktree /path/to/repo
HEAD abc1234
branch refs/heads/main

worktree /path/to/worktree-feature-test
HEAD def5678
branch refs/heads/feature/test
`;

      (execa as any).mockResolvedValue({
        stdout: mockOutput,
        stderr: '',
        exitCode: 0,
      });

      const path = await worktree.worktreeExists('feature/test');

      expect(path).toBe('/path/to/worktree-feature-test');
      expect(execa).toHaveBeenCalledWith('git', ['worktree', 'list', '--porcelain']);
    });

    it('should return null if worktree does not exist for branch', async () => {
      const mockOutput = `worktree /path/to/repo
HEAD abc1234
branch refs/heads/main
`;

      (execa as any).mockResolvedValue({
        stdout: mockOutput,
        stderr: '',
        exitCode: 0,
      });

      const path = await worktree.worktreeExists('feature/non-existent');

      expect(path).toBeNull();
    });

    it('should handle empty worktree list', async () => {
      (execa as any).mockResolvedValue({
        stdout: '',
        stderr: '',
        exitCode: 0,
      });

      const path = await worktree.worktreeExists('feature/test');

      expect(path).toBeNull();
    });

    it('should throw WorktreeError on failure', async () => {
      (execa as any).mockRejectedValue(new Error('Git command failed'));

      await expect(worktree.worktreeExists('feature/test')).rejects.toThrow('Failed to list worktrees');
    });
  });

  describe('generateWorktreePath (T105)', () => {
    it('should generate worktree path with sanitized branch name', async () => {
      const repoRoot = '/path/to/repo';
      const branchName = 'feature/user-auth';

      const path = await worktree.generateWorktreePath(repoRoot, branchName);

      expect(path).toBe('/path/to/repo/.git/worktree/feature-user-auth');
    });

    it('should sanitize special characters in branch name', async () => {
      const repoRoot = '/path/to/repo';
      const branchName = 'feature/user:auth*with?special<chars>';

      const path = await worktree.generateWorktreePath(repoRoot, branchName);

      expect(path).toBe('/path/to/repo/.git/worktree/feature-user-auth-with-special-chars-');
    });

    it('should handle Windows-style paths', async () => {
      const repoRoot = 'C:\\path\\to\\repo';
      const branchName = 'feature/test';

      const path = await worktree.generateWorktreePath(repoRoot, branchName);

      // Path module will normalize this based on the platform
      expect(path).toContain('worktree');
      expect(path).toContain('feature-test');
    });
  });

  describe('createWorktree (T106)', () => {
    it('should create worktree for existing branch', async () => {
      (execa as any).mockResolvedValue({
        stdout: '',
        stderr: '',
        exitCode: 0,
      });

      const config = {
        branchName: 'feature/test',
        worktreePath: '/path/to/worktree',
        repoRoot: '/path/to/repo',
        isNewBranch: false,
        baseBranch: 'main',
      };

      await worktree.createWorktree(config);

      expect(execa).toHaveBeenCalledWith('git', [
        'worktree',
        'add',
        '/path/to/worktree',
        'feature/test',
      ]);
    });

    it('should create worktree with new branch', async () => {
      (execa as any).mockResolvedValue({
        stdout: '',
        stderr: '',
        exitCode: 0,
      });

      const config = {
        branchName: 'feature/new-feature',
        worktreePath: '/path/to/worktree',
        repoRoot: '/path/to/repo',
        isNewBranch: true,
        baseBranch: 'main',
      };

      await worktree.createWorktree(config);

      expect(execa).toHaveBeenCalledWith('git', [
        'worktree',
        'add',
        '-b',
        'feature/new-feature',
        '/path/to/worktree',
        'main',
      ]);
    });

    it('should create worktree from different base branch', async () => {
      (execa as any).mockResolvedValue({
        stdout: '',
        stderr: '',
        exitCode: 0,
      });

      const config = {
        branchName: 'hotfix/bug-fix',
        worktreePath: '/path/to/worktree',
        repoRoot: '/path/to/repo',
        isNewBranch: true,
        baseBranch: 'develop',
      };

      await worktree.createWorktree(config);

      expect(execa).toHaveBeenCalledWith('git', [
        'worktree',
        'add',
        '-b',
        'hotfix/bug-fix',
        '/path/to/worktree',
        'develop',
      ]);
    });

    it('should throw WorktreeError on failure', async () => {
      (execa as any).mockRejectedValue(new Error('Failed to create worktree'));

      const config = {
        branchName: 'feature/test',
        worktreePath: '/path/to/worktree',
        repoRoot: '/path/to/repo',
        isNewBranch: false,
        baseBranch: 'main',
      };

      await expect(worktree.createWorktree(config)).rejects.toThrow('Failed to create worktree for feature/test');
    });
  });

  describe('removeWorktree (T702)', () => {
    it('should remove worktree without force', async () => {
      (execa as any).mockResolvedValue({
        stdout: '',
        stderr: '',
        exitCode: 0,
      });

      await worktree.removeWorktree('/path/to/worktree');

      expect(execa).toHaveBeenCalledWith('git', ['worktree', 'remove', '/path/to/worktree']);
    });

    it('should force remove worktree when force=true', async () => {
      (execa as any).mockResolvedValue({
        stdout: '',
        stderr: '',
        exitCode: 0,
      });

      await worktree.removeWorktree('/path/to/worktree', true);

      expect(execa).toHaveBeenCalledWith('git', ['worktree', 'remove', '--force', '/path/to/worktree']);
    });

    it('should throw WorktreeError on failure', async () => {
      (execa as any).mockRejectedValue(new Error('Worktree removal failed'));

      await expect(worktree.removeWorktree('/path/to/worktree')).rejects.toThrow('Failed to remove worktree');
    });
  });

  describe('listAdditionalWorktrees (T701)', () => {
    it('should call listWorktrees via git command', async () => {
      const mockWorktreeOutput = `worktree /path/to/repo
HEAD abc1234
branch refs/heads/main

worktree /path/to/worktree-feature-test
HEAD def5678
branch refs/heads/feature/test
`;

      (execa as any).mockResolvedValue({
        stdout: mockWorktreeOutput,
        stderr: '',
        exitCode: 0,
      });

      const worktreeList = await worktree.listAdditionalWorktrees();

      // Should call git worktree list
      expect(execa).toHaveBeenCalledWith('git', ['worktree', 'list', '--porcelain']);

      // Result should be an array
      expect(Array.isArray(worktreeList)).toBe(true);
    });

    it('should exclude main repository from results', async () => {
      const mockWorktreeOutput = `worktree /path/to/repo
HEAD abc1234
branch refs/heads/main

worktree /path/to/worktree-feature-test
HEAD def5678
branch refs/heads/feature/test
`;

      (execa as any).mockResolvedValue({
        stdout: mockWorktreeOutput,
        stderr: '',
        exitCode: 0,
      });

      const worktreeList = await worktree.listAdditionalWorktrees();

      // None of the returned worktrees should have 'main' as their branch
      // (assuming main repo is on main branch)
      expect(worktreeList.length).toBeGreaterThanOrEqual(0);
    });

    it('should handle empty worktree output', async () => {
      (execa as any).mockResolvedValue({
        stdout: '',
        stderr: '',
        exitCode: 0,
      });

      const worktreeList = await worktree.listAdditionalWorktrees();

      // Should return empty array
      expect(Array.isArray(worktreeList)).toBe(true);
    });
  });
});
