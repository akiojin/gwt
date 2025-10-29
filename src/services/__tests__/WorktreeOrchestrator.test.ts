import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  WorktreeOrchestrator,
  type WorktreeService,
} from "../WorktreeOrchestrator.js";
import * as git from "../../git.js";

// Mock git.getCurrentBranch
vi.mock("../../git.js", () => ({
  getCurrentBranch: vi.fn(),
}));

describe("WorktreeOrchestrator", () => {
  let orchestrator: WorktreeOrchestrator;
  let mockWorktreeService: WorktreeService;
  const mockRepoRoot = "/mock/repo";
  const mockBranch = "feature-test";
  const mockWorktreePath = "/mock/repo/.git/worktree/feature-test";

  beforeEach(() => {
    // Create mock service without vi.mock()
    mockWorktreeService = {
      worktreeExists: vi.fn(),
      generateWorktreePath: vi.fn(),
      createWorktree: vi.fn(),
    };
    orchestrator = new WorktreeOrchestrator(mockWorktreeService);
  });

  describe("ensureWorktree", () => {
    it("should return existing worktree path if worktree exists", async () => {
      // Arrange
      (mockWorktreeService.worktreeExists as any).mockResolvedValue(
        mockWorktreePath,
      );

      // Act
      const result = await orchestrator.ensureWorktree(
        mockBranch,
        mockRepoRoot,
      );

      // Assert
      expect(result).toBe(mockWorktreePath);
      expect(mockWorktreeService.worktreeExists).toHaveBeenCalledWith(
        mockBranch,
      );
      expect(mockWorktreeService.generateWorktreePath).not.toHaveBeenCalled();
      expect(mockWorktreeService.createWorktree).not.toHaveBeenCalled();
    });

    it("should create new worktree if it does not exist", async () => {
      // Arrange
      (mockWorktreeService.worktreeExists as any).mockResolvedValue(null);
      (mockWorktreeService.generateWorktreePath as any).mockResolvedValue(
        mockWorktreePath,
      );
      (mockWorktreeService.createWorktree as any).mockResolvedValue(undefined);

      // Act
      const result = await orchestrator.ensureWorktree(
        mockBranch,
        mockRepoRoot,
      );

      // Assert
      expect(result).toBe(mockWorktreePath);
      expect(mockWorktreeService.worktreeExists).toHaveBeenCalledWith(
        mockBranch,
      );
      expect(mockWorktreeService.generateWorktreePath).toHaveBeenCalledWith(
        mockRepoRoot,
        mockBranch,
      );
      expect(mockWorktreeService.createWorktree).toHaveBeenCalledWith({
        branchName: mockBranch,
        worktreePath: mockWorktreePath,
        repoRoot: mockRepoRoot,
        isNewBranch: false,
        baseBranch: "main",
      });
    });

    it("should use custom base branch when provided", async () => {
      // Arrange
      (mockWorktreeService.worktreeExists as any).mockResolvedValue(null);
      (mockWorktreeService.generateWorktreePath as any).mockResolvedValue(
        mockWorktreePath,
      );
      (mockWorktreeService.createWorktree as any).mockResolvedValue(undefined);
      const customBaseBranch = "develop";

      // Act
      const result = await orchestrator.ensureWorktree(mockBranch, mockRepoRoot, {
        baseBranch: customBaseBranch,
      });

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

    it("should mark worktree creation as new branch when requested", async () => {
      (mockWorktreeService.worktreeExists as any).mockResolvedValue(null);
      (mockWorktreeService.generateWorktreePath as any).mockResolvedValue(
        mockWorktreePath,
      );

      (mockWorktreeService.createWorktree as any).mockResolvedValue(undefined);

      const result = await orchestrator.ensureWorktree(mockBranch, mockRepoRoot, {
        baseBranch: "origin/feature-test",
        isNewBranch: true,
      });

      expect(result).toBe(mockWorktreePath);
      expect(mockWorktreeService.createWorktree).toHaveBeenCalledWith({
        branchName: mockBranch,
        worktreePath: mockWorktreePath,
        repoRoot: mockRepoRoot,
        isNewBranch: true,
        baseBranch: "origin/feature-test",
      });
    });

    it("should throw error if worktree creation fails", async () => {
      // Arrange
      (mockWorktreeService.worktreeExists as any).mockResolvedValue(null);
      (mockWorktreeService.generateWorktreePath as any).mockResolvedValue(
        mockWorktreePath,
      );
      const mockError = new Error("Failed to create worktree");
      (mockWorktreeService.createWorktree as any).mockRejectedValue(mockError);

      // Act & Assert
      await expect(
        orchestrator.ensureWorktree(mockBranch, mockRepoRoot),
      ).rejects.toThrow("Failed to create worktree");
    });

    it("should reuse existing worktree if creation reports branch already checked out", async () => {
      const existingPath = "/mock/repo/.git/worktree/feature-test-existing";

      (mockWorktreeService.worktreeExists as any)
        .mockResolvedValueOnce(null)
        .mockResolvedValueOnce(existingPath);

      (mockWorktreeService.generateWorktreePath as any).mockResolvedValue(
        mockWorktreePath,
      );

      const alreadyCheckedOutError = new Error(
        "fatal: 'feature-test' is already checked out at '/mock/repo/.git/worktree/feature-test-existing'",
      );

      (mockWorktreeService.createWorktree as any).mockRejectedValue(
        alreadyCheckedOutError,
      );

      const result = await orchestrator.ensureWorktree(
        mockBranch,
        mockRepoRoot,
      );

      expect(result).toBe(existingPath);
      expect(mockWorktreeService.worktreeExists).toHaveBeenCalledTimes(2);
      expect(mockWorktreeService.createWorktree).toHaveBeenCalledTimes(1);
    });

    describe("current branch handling", () => {
      it("should return repository root when current branch is selected", async () => {
        // Arrange
        const currentBranch = "main";
        (git.getCurrentBranch as any).mockResolvedValue(currentBranch);

        // Act
        const result = await orchestrator.ensureWorktree(
          currentBranch,
          mockRepoRoot,
        );

        // Assert
        expect(result).toBe(mockRepoRoot);
        expect(git.getCurrentBranch).toHaveBeenCalled();
        expect(mockWorktreeService.worktreeExists).not.toHaveBeenCalled();
        expect(mockWorktreeService.createWorktree).not.toHaveBeenCalled();
      });

      it("should create worktree when non-current branch is selected", async () => {
        // Arrange
        const currentBranch = "main";
        const otherBranch = "feature-test";
        (git.getCurrentBranch as any).mockResolvedValue(currentBranch);
        (mockWorktreeService.worktreeExists as any).mockResolvedValue(null);
        (mockWorktreeService.generateWorktreePath as any).mockResolvedValue(
          mockWorktreePath,
        );
        (mockWorktreeService.createWorktree as any).mockResolvedValue(undefined);

        // Act
        const result = await orchestrator.ensureWorktree(
          otherBranch,
          mockRepoRoot,
        );

        // Assert
        expect(result).toBe(mockWorktreePath);
        expect(git.getCurrentBranch).toHaveBeenCalled();
        expect(mockWorktreeService.worktreeExists).toHaveBeenCalledWith(
          otherBranch,
        );
        expect(mockWorktreeService.createWorktree).toHaveBeenCalled();
      });

      it("should fallback to normal flow when getCurrentBranch returns null", async () => {
        // Arrange
        (git.getCurrentBranch as any).mockResolvedValue(null);
        (mockWorktreeService.worktreeExists as any).mockResolvedValue(null);
        (mockWorktreeService.generateWorktreePath as any).mockResolvedValue(
          mockWorktreePath,
        );
        (mockWorktreeService.createWorktree as any).mockResolvedValue(undefined);

        // Act
        const result = await orchestrator.ensureWorktree(
          mockBranch,
          mockRepoRoot,
        );

        // Assert
        expect(result).toBe(mockWorktreePath);
        expect(git.getCurrentBranch).toHaveBeenCalled();
        expect(mockWorktreeService.worktreeExists).toHaveBeenCalledWith(
          mockBranch,
        );
        expect(mockWorktreeService.createWorktree).toHaveBeenCalled();
      });
    });
  });
});
