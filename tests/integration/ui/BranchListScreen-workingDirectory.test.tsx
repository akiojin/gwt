import { describe, it, expect, vi } from "vitest";
import { render } from "ink-testing-library";
import React from "react";
import { BranchListScreen } from "../../../src/cli/ui/components/screens/BranchListScreen.js";
import type { BranchItem, Statistics } from "../../../src/cli/ui/types.js";

describe("BranchListScreen - Working Directory Integration", () => {
  const mockStats: Statistics = {
    localCount: 5,
    remoteCount: 10,
    worktreeCount: 2,
    changesCount: 1,
    lastUpdated: new Date(),
  };

  const mockBranches: BranchItem[] = [
    {
      name: "main",
      remote: null,
      head: true,
      worktreePath: null,
      ahead: 0,
      behind: 0,
      icons: ["★"],
      hasChanges: false,
      label: "main",
      value: "main",
    },
  ];

  const mockOnSelect = vi.fn();

  describe("Working Directory Propagation", () => {
    it("should pass working directory to Header component", () => {
      const testDir = "/home/user/test-project";
      const { lastFrame } = render(
        <BranchListScreen
          branches={mockBranches}
          stats={mockStats}
          onSelect={mockOnSelect}
          workingDirectory={testDir}
        />
      );
      const output = lastFrame();

      // Should display working directory in header section
      expect(output).toContain("Working Directory:");
      expect(output).toContain(testDir);
    });

    it("should not display working directory when undefined", () => {
      const { lastFrame } = render(
        <BranchListScreen
          branches={mockBranches}
          stats={mockStats}
          onSelect={mockOnSelect}
          workingDirectory={undefined}
        />
      );
      const output = lastFrame();

      expect(output).not.toContain("Working Directory:");
    });

    it("should display working directory along with version", () => {
      const testDir = "/var/www/app";
      const testVersion = "1.17.0";
      const { lastFrame } = render(
        <BranchListScreen
          branches={mockBranches}
          stats={mockStats}
          onSelect={mockOnSelect}
          version={testVersion}
          workingDirectory={testDir}
        />
      );
      const output = lastFrame();

      expect(output).toContain(`v${testVersion}`);
      expect(output).toContain("Working Directory:");
      expect(output).toContain(testDir);
    });
  });

  describe("Layout Integration", () => {
    it("should render working directory between divider and stats", () => {
      const testDir = "/test/integration/path";
      const { lastFrame } = render(
        <BranchListScreen
          branches={mockBranches}
          stats={mockStats}
          onSelect={mockOnSelect}
          workingDirectory={testDir}
        />
      );
      const output = lastFrame();
      const lines = output.split("\n");

      // Find key sections
      const titleLineIndex = lines.findIndex((line) =>
        line.includes("gwt")
      );
      const dividerLineIndex = lines.findIndex((line) => line.includes("─"));
      const workingDirLineIndex = lines.findIndex((line) =>
        line.includes("Working Directory:")
      );
      const statsLineIndex = lines.findIndex((line) => line.includes("Local:"));

      // Verify order
      expect(titleLineIndex).toBeGreaterThanOrEqual(0);
      expect(dividerLineIndex).toBeGreaterThan(titleLineIndex);
      expect(workingDirLineIndex).toBeGreaterThan(dividerLineIndex);
      expect(statsLineIndex).toBeGreaterThan(workingDirLineIndex);
    });

    it("should maintain proper spacing with working directory", () => {
      const testDir = "/spacing/test";
      const { lastFrame } = render(
        <BranchListScreen
          branches={mockBranches}
          stats={mockStats}
          onSelect={mockOnSelect}
          workingDirectory={testDir}
        />
      );
      const output = lastFrame();

      // Should have all main sections
      expect(output).toContain("gwt");
      expect(output).toContain("Working Directory:");
      expect(output).toContain("Local:");
      expect(output).toContain("Remote:");
    });
  });

  describe("Real-world Scenarios", () => {
    it("should handle typical project directory path", () => {
      const projectPath = "/home/developer/projects/my-app";
      const { lastFrame } = render(
        <BranchListScreen
          branches={mockBranches}
          stats={mockStats}
          onSelect={mockOnSelect}
          workingDirectory={projectPath}
          version="1.17.0"
        />
      );
      const output = lastFrame();

      expect(output).toContain("Working Directory:");
      expect(output).toContain(projectPath);
      expect(output).toContain("v1.17.0");
    });

    it("should handle deep directory hierarchy", () => {
      const deepPath =
        "/home/user/development/projects/client/application/backend/src";
      const { lastFrame } = render(
        <BranchListScreen
          branches={mockBranches}
          stats={mockStats}
          onSelect={mockOnSelect}
          workingDirectory={deepPath}
        />
      );
      const output = lastFrame();

      expect(output).toContain("Working Directory:");
      expect(output).toContain(deepPath);
    });

    it("should handle path with spaces", () => {
      const pathWithSpaces = "/Users/developer/My Projects/App";
      const { lastFrame } = render(
        <BranchListScreen
          branches={mockBranches}
          stats={mockStats}
          onSelect={mockOnSelect}
          workingDirectory={pathWithSpaces}
        />
      );
      const output = lastFrame();

      expect(output).toContain("Working Directory:");
      expect(output).toContain(pathWithSpaces);
    });

    it("should work with typical worktree path", () => {
      const worktreePath = "/repo/.worktrees/feature-branch";
      const { lastFrame } = render(
        <BranchListScreen
          branches={mockBranches}
          stats={mockStats}
          onSelect={mockOnSelect}
          workingDirectory={worktreePath}
        />
      );
      const output = lastFrame();

      expect(output).toContain("Working Directory:");
      expect(output).toContain(worktreePath);
    });
  });

  describe("Props Consistency", () => {
    it("should handle all props together without conflicts", () => {
      const { lastFrame } = render(
        <BranchListScreen
          branches={mockBranches}
          stats={mockStats}
          onSelect={mockOnSelect}
          onNavigate={vi.fn()}
          onQuit={vi.fn()}
          onRefresh={vi.fn()}
          loading={false}
          error={null}
          lastUpdated={new Date()}
          version="1.17.0"
          workingDirectory="/full/props/test"
        />
      );
      const output = lastFrame();

      expect(output).toContain("gwt");
      expect(output).toContain("v1.17.0");
      expect(output).toContain("Working Directory:");
      expect(output).toContain("/full/props/test");
      expect(output).toContain("Local:");
    });
  });
});
