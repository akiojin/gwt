import { describe, it, expect } from "vitest";
import { render } from "ink-testing-library";
import React from "react";
import { Header } from "../../../../src/cli/ui/components/parts/Header.js";

describe("Header Component", () => {
  describe("Basic Rendering", () => {
    it("should render title correctly", () => {
      const { lastFrame } = render(<Header title="Test Title" />);
      expect(lastFrame()).toContain("Test Title");
    });

    it("should render title with version", () => {
      const { lastFrame } = render(
        <Header title="Test App" version="1.0.0" />
      );
      expect(lastFrame()).toContain("Test App v1.0.0");
    });

    it("should render divider when showDivider is true", () => {
      const { lastFrame } = render(
        <Header title="Test" showDivider={true} dividerChar="─" width={10} />
      );
      const output = lastFrame();
      expect(output).toContain("─");
    });

    it("should not render divider when showDivider is false", () => {
      const { lastFrame } = render(
        <Header title="Test" showDivider={false} />
      );
      const output = lastFrame();
      // Should only have title, no divider
      expect(output).toContain("Test");
      expect(output.split("\n").length).toBeLessThan(3);
    });
  });

  describe("Working Directory Display", () => {
    it("should display working directory when provided", () => {
      const testDir = "/home/user/project";
      const { lastFrame } = render(
        <Header title="Test" workingDirectory={testDir} />
      );
      const output = lastFrame();

      expect(output).toContain("Working Directory:");
      expect(output).toContain(testDir);
    });

    it("should not display working directory when undefined", () => {
      const { lastFrame } = render(
        <Header title="Test" workingDirectory={undefined} />
      );
      const output = lastFrame();

      expect(output).not.toContain("Working Directory:");
    });

    it("should display working directory with absolute path", () => {
      const absolutePath = "/var/www/application";
      const { lastFrame } = render(
        <Header title="App" version="2.0.0" workingDirectory={absolutePath} />
      );
      const output = lastFrame();

      expect(output).toContain("Working Directory:");
      expect(output).toContain(absolutePath);
      expect(output).toContain("App v2.0.0");
    });

    it("should display working directory with long path", () => {
      const longPath =
        "/home/user/development/projects/client-name/application/backend/src";
      const { lastFrame } = render(
        <Header title="Test" workingDirectory={longPath} />
      );
      const output = lastFrame();

      expect(output).toContain("Working Directory:");
      expect(output).toContain(longPath);
    });

    it("should display working directory with special characters", () => {
      const pathWithSpaces = "/home/user/my project/app";
      const { lastFrame } = render(
        <Header title="Test" workingDirectory={pathWithSpaces} />
      );
      const output = lastFrame();

      expect(output).toContain("Working Directory:");
      expect(output).toContain(pathWithSpaces);
    });
  });

  describe("Layout Order", () => {
    it("should render elements in correct order: title, divider, working directory", () => {
      const { lastFrame } = render(
        <Header
          title="Claude Worktree"
          version="1.0.0"
          workingDirectory="/test/path"
          showDivider={true}
          dividerChar="─"
        />
      );
      const output = lastFrame();
      const lines = output.split("\n").filter((line) => line.trim() !== "");

      // First line: title with version
      expect(lines[0]).toContain("Claude Worktree v1.0.0");

      // Second line: divider
      expect(lines[1]).toContain("─");

      // Third line: working directory
      expect(lines[2]).toContain("Working Directory:");
      expect(lines[2]).toContain("/test/path");
    });

    it("should render only title and working directory when divider is hidden", () => {
      const { lastFrame } = render(
        <Header
          title="Test"
          workingDirectory="/path"
          showDivider={false}
        />
      );
      const output = lastFrame();
      const lines = output.split("\n").filter((line) => line.trim() !== "");

      expect(lines[0]).toContain("Test");
      expect(lines[1]).toContain("Working Directory:");
    });
  });

  describe("Edge Cases", () => {
    it("should handle empty string as working directory", () => {
      const { lastFrame } = render(
        <Header title="Test" workingDirectory="" />
      );
      const output = lastFrame();

      // Empty string is falsy, should not display
      expect(output).not.toContain("Working Directory:");
    });

    it("should handle null version", () => {
      const { lastFrame } = render(
        <Header title="Test" version={null} workingDirectory="/path" />
      );
      const output = lastFrame();

      expect(output).toContain("Test");
      expect(output).not.toContain("null");
      expect(output).toContain("Working Directory:");
    });

    it("should handle all props together", () => {
      const { lastFrame } = render(
        <Header
          title="Full Test"
          titleColor="cyan"
          version="3.0.0"
          workingDirectory="/complete/path"
          showDivider={true}
          dividerChar="═"
          width={20}
        />
      );
      const output = lastFrame();

      expect(output).toContain("Full Test v3.0.0");
      expect(output).toContain("═");
      expect(output).toContain("Working Directory:");
      expect(output).toContain("/complete/path");
    });
  });
});
