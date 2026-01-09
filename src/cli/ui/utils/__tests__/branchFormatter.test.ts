import { describe, expect, it } from "bun:test";
import {
  formatBranchItem,
  getLatestActivityTimestamp,
} from "../branchFormatter.js";
import type { BranchInfo } from "../../types.js";

// TDD: テストを先に書き、実装は後から行う

const LOCAL_DATE_TIME_FORMATTER = new Intl.DateTimeFormat(undefined, {
  year: "numeric",
  month: "2-digit",
  day: "2-digit",
  hour: "2-digit",
  minute: "2-digit",
  hour12: false,
});

const formatLocalDateTime = (timestampMs: number): string => {
  const date = new Date(timestampMs);
  const parts = LOCAL_DATE_TIME_FORMATTER.formatToParts(date);
  const get = (type: Intl.DateTimeFormatPartTypes) =>
    parts.find((part) => part.type === type)?.value;
  const year = get("year");
  const month = get("month");
  const day = get("day");
  const hour = get("hour");
  const minute = get("minute");
  if (!year || !month || !day || !hour || !minute) {
    return LOCAL_DATE_TIME_FORMATTER.format(date);
  }
  return `${year}-${month}-${day} ${hour}:${minute}`;
};

describe("branchFormatter", () => {
  describe("buildLastToolUsageLabel with version", () => {
    const baseBranch: BranchInfo = {
      name: "feature/test",
      type: "local",
      isCurrent: false,
      branchType: "feature",
      hasRemoteCounterpart: false,
    };

    it("should format as 'ToolName@1.0.3 | 2024-01-08 12:00' with version", () => {
      const branch: BranchInfo = {
        ...baseBranch,
        lastToolUsage: {
          branch: "feature/test",
          worktreePath: "/path/to/worktree",
          toolId: "claude-code",
          toolLabel: "Claude Code",
          timestamp: new Date("2024-01-08T12:00:00").getTime(),
          toolVersion: "1.0.3",
        },
      };

      const item = formatBranchItem(branch);
      expect(item.lastToolUsageLabel).toBe(
        `Claude@1.0.3 | ${formatLocalDateTime(
          new Date("2024-01-08T12:00:00").getTime(),
        )}`,
      );
    });

    it("should format as 'ToolName@latest | 2024-01-08 12:00' when version is null", () => {
      const branch: BranchInfo = {
        ...baseBranch,
        lastToolUsage: {
          branch: "feature/test",
          worktreePath: "/path/to/worktree",
          toolId: "claude-code",
          toolLabel: "Claude Code",
          timestamp: new Date("2024-01-08T12:00:00").getTime(),
          toolVersion: null,
        },
      };

      const item = formatBranchItem(branch);
      expect(item.lastToolUsageLabel).toBe(
        `Claude@latest | ${formatLocalDateTime(
          new Date("2024-01-08T12:00:00").getTime(),
        )}`,
      );
    });

    it("should format as 'ToolName@latest | 2024-01-08 12:00' when version is undefined", () => {
      const branch: BranchInfo = {
        ...baseBranch,
        lastToolUsage: {
          branch: "feature/test",
          worktreePath: "/path/to/worktree",
          toolId: "claude-code",
          toolLabel: "Claude Code",
          timestamp: new Date("2024-01-08T12:00:00").getTime(),
        },
      };

      const item = formatBranchItem(branch);
      expect(item.lastToolUsageLabel).toBe(
        `Claude@latest | ${formatLocalDateTime(
          new Date("2024-01-08T12:00:00").getTime(),
        )}`,
      );
    });

    it("should format Codex with version", () => {
      const branch: BranchInfo = {
        ...baseBranch,
        lastToolUsage: {
          branch: "feature/test",
          worktreePath: "/path/to/worktree",
          toolId: "codex-cli",
          toolLabel: "Codex CLI",
          timestamp: new Date("2024-01-08T15:30:00").getTime(),
          toolVersion: "2.1.0-beta.1",
        },
      };

      const item = formatBranchItem(branch);
      expect(item.lastToolUsageLabel).toBe(
        `Codex@2.1.0-beta.1 | ${formatLocalDateTime(
          new Date("2024-01-08T15:30:00").getTime(),
        )}`,
      );
    });

    it("should format Gemini with version", () => {
      const branch: BranchInfo = {
        ...baseBranch,
        lastToolUsage: {
          branch: "feature/test",
          worktreePath: "/path/to/worktree",
          toolId: "gemini-cli",
          toolLabel: "Gemini CLI",
          timestamp: new Date("2024-01-08T09:15:00").getTime(),
          toolVersion: "0.5.0",
        },
      };

      const item = formatBranchItem(branch);
      expect(item.lastToolUsageLabel).toBe(
        `Gemini@0.5.0 | ${formatLocalDateTime(
          new Date("2024-01-08T09:15:00").getTime(),
        )}`,
      );
    });

    it("should format custom tool with version", () => {
      const branch: BranchInfo = {
        ...baseBranch,
        lastToolUsage: {
          branch: "feature/test",
          worktreePath: "/path/to/worktree",
          toolId: "custom-tool",
          toolLabel: "MyTool",
          timestamp: new Date("2024-01-08T10:00:00").getTime(),
          toolVersion: "1.0.0",
        },
      };

      const item = formatBranchItem(branch);
      expect(item.lastToolUsageLabel).toBe(
        `MyTool@1.0.0 | ${formatLocalDateTime(
          new Date("2024-01-08T10:00:00").getTime(),
        )}`,
      );
    });

    it("should return null when lastToolUsage is undefined", () => {
      const branch: BranchInfo = {
        ...baseBranch,
        lastToolUsage: undefined,
      };

      const item = formatBranchItem(branch);
      expect(item.lastToolUsageLabel).toBeNull();
    });

    it("should return null when lastToolUsage is null", () => {
      const branch: BranchInfo = {
        ...baseBranch,
        lastToolUsage: null,
      };

      const item = formatBranchItem(branch);
      expect(item.lastToolUsageLabel).toBeNull();
    });
  });

  describe("getLatestActivityTimestamp", () => {
    it("should return git timestamp when tool usage is undefined", () => {
      const branch: BranchInfo = {
        name: "feature/test",
        type: "local",
        isCurrent: false,
        branchType: "feature",
        hasRemoteCounterpart: false,
        latestCommitTimestamp: 1704700800, // 2024-01-08 12:00:00 UTC in seconds
      };

      const result = getLatestActivityTimestamp(branch);
      expect(result).toBe(1704700800);
    });

    it("should return tool timestamp when it is newer than git timestamp", () => {
      const branch: BranchInfo = {
        name: "feature/test",
        type: "local",
        isCurrent: false,
        branchType: "feature",
        hasRemoteCounterpart: false,
        latestCommitTimestamp: 1704700800, // 2024-01-08 12:00:00 UTC in seconds
        lastToolUsage: {
          branch: "feature/test",
          worktreePath: "/path/to/worktree",
          toolId: "claude-code",
          toolLabel: "Claude Code",
          timestamp: 1704704400000, // 2024-01-08 13:00:00 UTC in milliseconds
        },
      };

      const result = getLatestActivityTimestamp(branch);
      expect(result).toBe(1704704400); // converted to seconds
    });
  });
});
