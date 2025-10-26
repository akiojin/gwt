import { describe, it, expect, vi, beforeEach } from 'vitest';
import { WorktreeOrchestrator } from './WorktreeOrchestrator.js';

// Mock worktree module
const mockWorktreeExists = vi.fn();
const mockGenerateWorktreePath = vi.fn();
const mockCreateWorktree = vi.fn();

vi.mock('../worktree.js', () => ({
  worktreeExists: mockWorktreeExists,
  generateWorktreePath: mockGenerateWorktreePath,
  createWorktree: mockCreateWorktree,
}));

describe('WorktreeOrchestrator', () => {
  let orchestrator: WorktreeOrchestrator;
  const mockRepoRoot = '/mock/repo';
  const mockBranch = 'feature-test';
  const mockWorktreePath = '/mock/repo/.git/worktree/feature-test';

  beforeEach(() => {
    orchestrator = new WorktreeOrchestrator();
    vi.clearAllMocks();
  });

  describe('ensureWorktree', () => {
    it('should return existing worktree path if worktree exists', async () => {
      // Arrange
      mockWorktreeExists.mockResolvedValue(mockWorktreePath);

      // Act
      const result = await orchestrator.ensureWorktree(mockBranch, mockRepoRoot);

      // Assert
      expect(result).toBe(mockWorktreePath);
      expect(mockWorktreeExists).toHaveBeenCalledWith(mockBranch);
      expect(mockGenerateWorktreePath).not.toHaveBeenCalled();
      expect(mockCreateWorktree).not.toHaveBeenCalled();
    });

    it('should create new worktree if it does not exist', async () => {
      // Arrange
      mockWorktreeExists.mockResolvedValue(null);
      mockGenerateWorktreePath.mockResolvedValue(mockWorktreePath);
      mockCreateWorktree.mockResolvedValue(undefined);

      // Act
      const result = await orchestrator.ensureWorktree(mockBranch, mockRepoRoot);

      // Assert
      expect(result).toBe(mockWorktreePath);
      expect(mockWorktreeExists).toHaveBeenCalledWith(mockBranch);
      expect(mockGenerateWorktreePath).toHaveBeenCalledWith(mockBranch, mockRepoRoot);
      expect(mockCreateWorktree).toHaveBeenCalledWith({
        branchName: mockBranch,
        worktreePath: mockWorktreePath,
        repoRoot: mockRepoRoot,
        isNewBranch: false,
        baseBranch: 'main',
      });
    });

    it('should use custom base branch when provided', async () => {
      // Arrange
      mockWorktreeExists.mockResolvedValue(null);
      mockGenerateWorktreePath.mockResolvedValue(mockWorktreePath);
      mockCreateWorktree.mockResolvedValue(undefined);
      const customBaseBranch = 'develop';

      // Act
      const result = await orchestrator.ensureWorktree(
        mockBranch,
        mockRepoRoot,
        customBaseBranch
      );

      // Assert
      expect(result).toBe(mockWorktreePath);
      expect(mockCreateWorktree).toHaveBeenCalledWith({
        branchName: mockBranch,
        worktreePath: mockWorktreePath,
        repoRoot: mockRepoRoot,
        isNewBranch: false,
        baseBranch: customBaseBranch,
      });
    });

    it('should throw error if worktree creation fails', async () => {
      // Arrange
      mockWorktreeExists.mockResolvedValue(null);
      mockGenerateWorktreePath.mockResolvedValue(mockWorktreePath);
      const mockError = new Error('Failed to create worktree');
      mockCreateWorktree.mockRejectedValue(mockError);

      // Act & Assert
      await expect(
        orchestrator.ensureWorktree(mockBranch, mockRepoRoot)
      ).rejects.toThrow('Failed to create worktree');
    });
  });
});
