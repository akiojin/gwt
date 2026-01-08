import { describe, it, expect, beforeEach, afterEach } from "bun:test";
import { mkdir, rm } from "fs/promises";
import { join } from "path";
import { tmpdir } from "os";
import { saveSession, loadSession } from "../index.js";

describe("saveSession", () => {
  let testDir: string;
  let testRepoRoot: string;

  beforeEach(async () => {
    // Create a unique temporary directory for each test
    testDir = join(
      tmpdir(),
      `gwt-test-${Date.now()}-${Math.random().toString(36).slice(2)}`,
    );
    testRepoRoot = join(testDir, "test-repo");
    await mkdir(testRepoRoot, { recursive: true });
  });

  afterEach(async () => {
    // Clean up temporary directory
    try {
      await rm(testDir, { recursive: true, force: true });
    } catch {
      // Ignore cleanup errors
    }
  });

  it("should save toolVersion to session history", async () => {
    // Arrange
    const sessionData = {
      lastWorktreePath: testRepoRoot,
      lastBranch: "feature/test",
      lastUsedTool: "claude-code",
      toolLabel: "Claude Code",
      mode: "normal" as const,
      model: "opus",
      reasoningLevel: "high",
      skipPermissions: false,
      toolVersion: "2.1.1",
      timestamp: Date.now(),
      repositoryRoot: testRepoRoot,
    };

    // Act
    await saveSession(sessionData);
    const loaded = await loadSession(testRepoRoot);

    // Assert
    expect(loaded).not.toBeNull();
    expect(loaded?.history).toHaveLength(1);
    expect(loaded?.history?.[0]?.toolVersion).toBe("2.1.1");
  });

  it("should save null toolVersion when not provided", async () => {
    // Arrange
    const sessionData = {
      lastWorktreePath: testRepoRoot,
      lastBranch: "feature/test",
      lastUsedTool: "claude-code",
      toolLabel: "Claude Code",
      mode: "normal" as const,
      model: "opus",
      timestamp: Date.now(),
      repositoryRoot: testRepoRoot,
    };

    // Act
    await saveSession(sessionData);
    const loaded = await loadSession(testRepoRoot);

    // Assert
    expect(loaded).not.toBeNull();
    expect(loaded?.history).toHaveLength(1);
    expect(loaded?.history?.[0]?.toolVersion).toBeNull();
  });

  it("should preserve toolVersion across multiple saves", async () => {
    // Arrange - first save with version 1.0.0
    await saveSession({
      lastWorktreePath: testRepoRoot,
      lastBranch: "feature/test",
      lastUsedTool: "claude-code",
      toolLabel: "Claude Code",
      toolVersion: "1.0.0",
      timestamp: Date.now() - 1000,
      repositoryRoot: testRepoRoot,
    });

    // Act - second save with version 2.0.0
    await saveSession({
      lastWorktreePath: testRepoRoot,
      lastBranch: "feature/test",
      lastUsedTool: "claude-code",
      toolLabel: "Claude Code",
      toolVersion: "2.0.0",
      timestamp: Date.now(),
      repositoryRoot: testRepoRoot,
    });

    const loaded = await loadSession(testRepoRoot);

    // Assert - should have 2 entries with different versions
    expect(loaded?.history).toHaveLength(2);
    // History is stored oldest first (newest at the end)
    expect(loaded?.history?.[0]?.toolVersion).toBe("1.0.0");
    expect(loaded?.history?.[1]?.toolVersion).toBe("2.0.0");
  });

  it("should not add to history when skipHistory is true", async () => {
    // Arrange - first save
    await saveSession({
      lastWorktreePath: testRepoRoot,
      lastBranch: "feature/test",
      lastUsedTool: "claude-code",
      toolLabel: "Claude Code",
      toolVersion: "1.0.0",
      timestamp: Date.now(),
      repositoryRoot: testRepoRoot,
    });

    // Act - second save with skipHistory
    await saveSession(
      {
        lastWorktreePath: testRepoRoot,
        lastBranch: "feature/test",
        lastUsedTool: "claude-code",
        toolLabel: "Claude Code",
        toolVersion: "2.0.0",
        timestamp: Date.now(),
        repositoryRoot: testRepoRoot,
      },
      { skipHistory: true },
    );

    const loaded = await loadSession(testRepoRoot);

    // Assert - should still have only 1 entry
    expect(loaded?.history).toHaveLength(1);
    expect(loaded?.history?.[0]?.toolVersion).toBe("1.0.0");
  });
});
