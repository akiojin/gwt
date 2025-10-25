import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { createBranchTable } from '../../../src/ui/table';
import { localBranches, remoteBranches } from '../../fixtures/branches';
import { worktrees } from '../../fixtures/worktrees';

// Mock dependencies
vi.mock('../../../src/git.js', () => ({
  getChangedFilesCount: vi.fn(),
}));

import { getChangedFilesCount } from '../../../src/git.js';

describe('table.ts - Branch Table Operations', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  describe('createBranchTable (T107)', () => {
    it('should create branch table with branches and worktrees', async () => {
      // Mock getChangedFilesCount to return 0 for all worktrees
      (getChangedFilesCount as any).mockResolvedValue(0);

      const choices = await createBranchTable(localBranches, worktrees);

      // Should include header and separator
      expect(choices.length).toBeGreaterThan(localBranches.length);

      // Check that header and separator are disabled
      const header = choices.find(c => c.value === '__header__');
      const separator = choices.find(c => c.value === '__separator__');

      expect(header).toBeDefined();
      expect(separator).toBeDefined();
      expect(header?.disabled).toBe(true);
      expect(separator?.disabled).toBe(true);
    });

    it('should include branch information in choices', async () => {
      (getChangedFilesCount as any).mockResolvedValue(0);

      const choices = await createBranchTable(localBranches, worktrees);

      // Find a specific branch
      const mainBranch = choices.find(c => c.value === 'main');
      expect(mainBranch).toBeDefined();
      expect(mainBranch?.name).toContain('main');
    });

    it('should show worktree status for branches', async () => {
      (getChangedFilesCount as any).mockResolvedValue(0);

      const choices = await createBranchTable(localBranches, worktrees);

      // Branches with worktrees should have worktree description
      const featureUserAuth = choices.find(c => c.value === 'feature/user-auth');
      expect(featureUserAuth?.description).toContain('Worktree');
    });

    it('should handle branches without worktrees', async () => {
      (getChangedFilesCount as any).mockResolvedValue(0);

      const branchesWithoutWorktrees = localBranches.filter(
        b => !worktrees.some(w => w.branch === b.name)
      );

      const choices = await createBranchTable(branchesWithoutWorktrees, []);

      const branch = choices.find(c => c.value === branchesWithoutWorktrees[0]?.name);
      expect(branch?.description).toContain('No worktree');
    });

    it('should display changed files count', async () => {
      // Mock some branches to have changes
      (getChangedFilesCount as any).mockImplementation(async (path: string) => {
        if (path.includes('user-auth')) return 5;
        return 0;
      });

      const choices = await createBranchTable(localBranches, worktrees);

      // The table should be created successfully
      expect(choices).toBeDefined();
      expect(choices.length).toBeGreaterThan(0);
    });

    it('should handle remote branches', async () => {
      (getChangedFilesCount as any).mockResolvedValue(0);

      const choices = await createBranchTable(remoteBranches, []);

      // Remote branches should not have worktrees
      const remoteBranch = choices.find(c => c.value === 'origin/main');
      expect(remoteBranch).toBeDefined();
      expect(remoteBranch?.description).toContain('No worktree');
    });

    it('should sort branches with current branch first', async () => {
      (getChangedFilesCount as any).mockResolvedValue(0);

      const choices = await createBranchTable(localBranches, worktrees);

      // Skip header and separator
      const branchChoices = choices.filter(
        c => c.value !== '__header__' && c.value !== '__separator__'
      );

      // Current branch should be first
      const currentBranch = localBranches.find(b => b.isCurrent);
      if (currentBranch) {
        expect(branchChoices[0]?.value).toBe(currentBranch.name);
      }
    });

    it('should filter out origin branch', async () => {
      (getChangedFilesCount as any).mockResolvedValue(0);

      const branchesWithOrigin = [
        ...localBranches,
        {
          name: 'origin',
          type: 'remote' as const,
          branchType: 'other' as const,
          isCurrent: false,
        },
      ];

      const choices = await createBranchTable(branchesWithOrigin, []);

      // origin branch should be filtered out
      const originBranch = choices.find(c => c.value === 'origin');
      expect(originBranch).toBeUndefined();
    });

    it('should handle inaccessible worktrees', async () => {
      (getChangedFilesCount as any).mockResolvedValue(0);

      const inaccessibleWorktrees = [
        {
          branch: 'feature/test',
          path: '/inaccessible/path',
          isAccessible: false,
        },
      ];

      const branches = [
        {
          name: 'feature/test',
          type: 'local' as const,
          branchType: 'feature' as const,
          isCurrent: false,
        },
      ];

      const choices = await createBranchTable(branches, inaccessibleWorktrees as any);

      // Should handle inaccessible worktrees without error
      expect(choices).toBeDefined();
      expect(choices.length).toBeGreaterThan(0);
    });

    it('should handle getChangedFilesCount errors gracefully', async () => {
      // Mock getChangedFilesCount to throw error
      (getChangedFilesCount as any).mockRejectedValue(new Error('Failed to get changes'));

      const choices = await createBranchTable(localBranches, worktrees);

      // Should still create table even if change detection fails
      expect(choices).toBeDefined();
      expect(choices.length).toBeGreaterThan(0);
    });
  });
});
