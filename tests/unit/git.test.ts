import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import * as git from '../../src/git';
import { localBranches, remoteBranches } from '../fixtures/branches';

// Mock execa
vi.mock('execa', () => ({
  execa: vi.fn(),
}));

import { execa } from 'execa';

describe('git.ts - Branch Operations', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  describe('getLocalBranches (T102)', () => {
    it('should return list of local branches', async () => {
      const mockOutput = `main
develop
feature/user-auth
feature/dashboard
hotfix/security-patch
release/1.2.0`;

      (execa as any).mockResolvedValue({
        stdout: mockOutput,
        stderr: '',
        exitCode: 0,
      });

      const branches = await git.getLocalBranches();

      expect(branches).toHaveLength(6);
      expect(branches[0]).toEqual({
        name: 'main',
        type: 'local',
        branchType: 'main',
        isCurrent: false,
      });
      expect(branches[2]).toEqual({
        name: 'feature/user-auth',
        type: 'local',
        branchType: 'feature',
        isCurrent: false,
      });
      expect(execa).toHaveBeenCalledWith('git', ['branch', '--format=%(refname:short)']);
    });

    it('should handle empty branch list', async () => {
      (execa as any).mockResolvedValue({
        stdout: '',
        stderr: '',
        exitCode: 0,
      } as any);

      const branches = await git.getLocalBranches();

      expect(branches).toHaveLength(0);
    });

    it('should throw GitError on failure', async () => {
      (execa as any).mockRejectedValue(new Error('Git command failed'));

      await expect(git.getLocalBranches()).rejects.toThrow('Failed to get local branches');
    });
  });

  describe('getRemoteBranches (T103)', () => {
    it('should return list of remote branches', async () => {
      const mockOutput = `origin/main
origin/develop
origin/feature/api-integration
origin/hotfix/bug-123`;

      (execa as any).mockResolvedValue({
        stdout: mockOutput,
        stderr: '',
        exitCode: 0,
      } as any);

      const branches = await git.getRemoteBranches();

      expect(branches).toHaveLength(4);
      expect(branches[0]).toEqual({
        name: 'origin/main',
        type: 'remote',
        branchType: 'main',
        isCurrent: false,
      });
      expect(branches[2]).toEqual({
        name: 'origin/feature/api-integration',
        type: 'remote',
        branchType: 'feature',
        isCurrent: false,
      });
      expect(execa).toHaveBeenCalledWith('git', ['branch', '-r', '--format=%(refname:short)']);
    });

    it('should filter out HEAD references', async () => {
      const mockOutput = `origin/HEAD -> origin/main
origin/main
origin/develop`;

      (execa as any).mockResolvedValue({
        stdout: mockOutput,
        stderr: '',
        exitCode: 0,
      } as any);

      const branches = await git.getRemoteBranches();

      expect(branches).toHaveLength(2);
      expect(branches.every(b => !b.name.includes('HEAD'))).toBe(true);
    });

    it('should throw GitError on failure', async () => {
      (execa as any).mockRejectedValue(new Error('Git command failed'));

      await expect(git.getRemoteBranches()).rejects.toThrow('Failed to get remote branches');
    });
  });

  describe('getAllBranches (T101)', () => {
    it('should return all local and remote branches', async () => {
      let callCount = 0;
      (execa as any).mockImplementation(async (command: string, args?: readonly string[]) => {
        callCount++;

        // getCurrentBranch call (check this first as it's most specific)
        if (args?.[0] === 'branch' && args.includes('--show-current')) {
          return {
            stdout: 'main',
            stderr: '',
            exitCode: 0,
          } as any;
        }

        // getRemoteBranches call
        if (args?.[0] === 'branch' && args.includes('-r')) {
          return {
            stdout: 'origin/main\norigin/develop',
            stderr: '',
            exitCode: 0,
          } as any;
        }

        // getLocalBranches call
        if (args?.[0] === 'branch' && args.includes('--format=%(refname:short)')) {
          return {
            stdout: 'main\ndevelop\nfeature/test',
            stderr: '',
            exitCode: 0,
          } as any;
        }

        return {
          stdout: '',
          stderr: '',
          exitCode: 0,
        } as any;
      });

      const branches = await git.getAllBranches();

      expect(branches).toHaveLength(5); // 3 local + 2 remote
      expect(branches.filter(b => b.type === 'local')).toHaveLength(3);
      expect(branches.filter(b => b.type === 'remote')).toHaveLength(2);

      // Check that current branch is marked
      const mainBranch = branches.find(b => b.name === 'main' && b.type === 'local');
      expect(mainBranch?.isCurrent).toBe(true);
    });

    it('should mark current branch as isCurrent', async () => {
      (execa as any).mockImplementation(async (command: string, args?: readonly string[]) => {
        if (args?.[0] === 'branch' && !args.includes('-r') && !args.includes('--show-current')) {
          return {
            stdout: 'main\nfeature/test',
            stderr: '',
            exitCode: 0,
          } as any;
        }

        if (args?.[0] === 'branch' && args.includes('-r')) {
          return {
            stdout: '',
            stderr: '',
            exitCode: 0,
          } as any;
        }

        if (args?.[0] === 'branch' && args.includes('--show-current')) {
          return {
            stdout: 'feature/test',
            stderr: '',
            exitCode: 0,
          } as any;
        }

        return {
          stdout: '',
          stderr: '',
          exitCode: 0,
        } as any;
      });

      const branches = await git.getAllBranches();

      const currentBranch = branches.find(b => b.name === 'feature/test');
      expect(currentBranch?.isCurrent).toBe(true);

      const mainBranch = branches.find(b => b.name === 'main');
      expect(mainBranch?.isCurrent).toBe(false);
    });

    it('should handle no current branch (detached HEAD)', async () => {
      (execa as any).mockImplementation(async (command: string, args?: readonly string[]) => {
        if (args?.[0] === 'branch' && !args.includes('-r') && !args.includes('--show-current')) {
          return {
            stdout: 'main',
            stderr: '',
            exitCode: 0,
          } as any;
        }

        if (args?.[0] === 'branch' && args.includes('-r')) {
          return {
            stdout: '',
            stderr: '',
            exitCode: 0,
          } as any;
        }

        if (args?.[0] === 'branch' && args.includes('--show-current')) {
          return {
            stdout: '',
            stderr: '',
            exitCode: 0,
          } as any;
        }

        return {
          stdout: '',
          stderr: '',
          exitCode: 0,
        } as any;
      });

      const branches = await git.getAllBranches();

      expect(branches.every(b => !b.isCurrent)).toBe(true);
    });
  });

  describe('branchExists (T201)', () => {
    it('should return true for existing branch', async () => {
      (execa as any).mockResolvedValue({
        stdout: '',
        stderr: '',
        exitCode: 0,
      } as any);

      const exists = await git.branchExists('main');

      expect(exists).toBe(true);
      expect(execa).toHaveBeenCalledWith('git', ['show-ref', '--verify', '--quiet', 'refs/heads/main']);
    });

    it('should return false for non-existent branch', async () => {
      (execa as any).mockRejectedValue(new Error('Branch not found'));

      const exists = await git.branchExists('non-existent');

      expect(exists).toBe(false);
    });
  });

  describe('createBranch (T201)', () => {
    it('should create a new branch', async () => {
      (execa as any).mockResolvedValue({
        stdout: '',
        stderr: '',
        exitCode: 0,
      } as any);

      await git.createBranch('feature/new-feature', 'main');

      expect(execa).toHaveBeenCalledWith('git', ['checkout', '-b', 'feature/new-feature', 'main']);
    });

    it('should use main as default base branch', async () => {
      (execa as any).mockResolvedValue({
        stdout: '',
        stderr: '',
        exitCode: 0,
      } as any);

      await git.createBranch('feature/new-feature');

      expect(execa).toHaveBeenCalledWith('git', ['checkout', '-b', 'feature/new-feature', 'main']);
    });

    it('should throw GitError on failure', async () => {
      (execa as any).mockRejectedValue(new Error('Failed to create branch'));

      await expect(git.createBranch('feature/test')).rejects.toThrow('Failed to create branch');
    });
  });

  describe('deleteBranch (T605)', () => {
    it('should delete a branch', async () => {
      (execa as any).mockResolvedValue({
        stdout: '',
        stderr: '',
        exitCode: 0,
      } as any);

      await git.deleteBranch('feature/old-feature');

      expect(execa).toHaveBeenCalledWith('git', ['branch', '-d', 'feature/old-feature']);
    });

    it('should force delete when force=true', async () => {
      (execa as any).mockResolvedValue({
        stdout: '',
        stderr: '',
        exitCode: 0,
      } as any);

      await git.deleteBranch('feature/old-feature', true);

      expect(execa).toHaveBeenCalledWith('git', ['branch', '-D', 'feature/old-feature']);
    });

    it('should throw GitError on failure', async () => {
      (execa as any).mockRejectedValue(new Error('Branch not found'));

      await expect(git.deleteBranch('feature/test')).rejects.toThrow('Failed to delete branch');
    });
  });
});
