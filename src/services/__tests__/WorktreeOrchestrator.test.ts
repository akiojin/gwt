import { describe, it, expect, vi, beforeEach } from 'vitest';
import { WorktreeOrchestrator, type WorktreeService } from '../WorktreeOrchestrator.js';

describe('WorktreeOrchestrator', () => {
  let orchestrator: WorktreeOrchestrator;
  let mockWorktreeService: WorktreeService;
  const mockRepoRoot = '/mock/repo';
  const mockBranch = 'feature-test';
  const mockWorktreePath = '/mock/repo/.git/worktree/feature-test';

  beforeEach(() => {
    // Create mock service without vi.mock()
    mockWorktreeService = {
      worktreeExists: vi.fn(),
      generateWorktreePath: vi.fn(),
      createWorktree: vi.fn(),
    };
    orchestrator = new WorktreeOrchestrator(mockWorktreeService);
  });

  describe('ensureWorktree', () => {
    it('should return existing worktree path if worktree exists', async () => {
      // Arrange
      (mockWorktreeService.worktreeExists as any).mockResolvedValue(mockWorktreePath);

      // Act
      const result = await orchestrator.ensureWorktree(mockBranch, mockRepoRoot);

      // Assert
      expect(result).toBe(mockWorktreePath);
      expect(mockWorktreeService.worktreeExists).toHaveBeenCalledWith(mockBranch);
      expect(mockWorktreeService.generateWorktreePath).not.toHaveBeenCalled();
      expect(mockWorktreeService.createWorktree).not.toHaveBeenCalled();
    });

    it('should create new worktree if it does not exist', async () => {
      // Arrange
      (mockWorktreeService.worktreeExists as any).mockResolvedValue(null);
      (mockWorktreeService.generateWorktreePath as any).mockResolvedValue(mockWorktreePath);
      (mockWorktreeService.createWorktree as any).mockResolvedValue(undefined);

      // Act
      const result = await orchestrator.ensureWorktree(mockBranch, mockRepoRoot);

      // Assert
      expect(result).toBe(mockWorktreePath);
      expect(mockWorktreeService.worktreeExists).toHaveBeenCalledWith(mockBranch);
      expect(mockWorktreeService.generateWorktreePath).toHaveBeenCalledWith(mockRepoRoot, mockBranch);
      expect(mockWorktreeService.createWorktree).toHaveBeenCalledWith({
        branchName: mockBranch,
        worktreePath: mockWorktreePath,
        repoRoot: mockRepoRoot,
        isNewBranch: false,
        baseBranch: 'main',
      });
    });

    it('should use custom base branch when provided', async () => {
      // Arrange
      (mockWorktreeService.worktreeExists as any).mockResolvedValue(null);
      (mockWorktreeService.generateWorktreePath as any).mockResolvedValue(mockWorktreePath);
      (mockWorktreeService.createWorktree as any).mockResolvedValue(undefined);
      const customBaseBranch = 'develop';

      // Act
      const result = await orchestrator.ensureWorktree(
        mockBranch,
        mockRepoRoot,
        customBaseBranch
      );

      // Assert
      expect(result).toBe(mockWorktreePath);
      expect(mockWorktreeService.createWorktree).toHaveBeenCalledWith({
        branchName: mockBranch,
        worktreePath: mockWorktreePath,
        repoRoot: mockRepoRoot,
        isNewBranch: false,
        baseBranch: customBaseBranch,
      });
    });

    it('should throw error if worktree creation fails', async () => {
      // Arrange
      (mockWorktreeService.worktreeExists as any).mockResolvedValue(null);
      (mockWorktreeService.generateWorktreePath as any).mockResolvedValue(mockWorktreePath);
      const mockError = new Error('Failed to create worktree');
      (mockWorktreeService.createWorktree as any).mockRejectedValue(mockError);

      // Act & Assert
      await expect(
        orchestrator.ensureWorktree(mockBranch, mockRepoRoot)
      ).rejects.toThrow('Failed to create worktree');
    });
  });
});
