import { describe, it, expect, beforeEach, afterEach } from "bun:test";
import fs from "node:fs";
import path from "node:path";
import {
  clearLogFiles,
  readLogLinesForDate,
  resolveLogDir,
  resolveLogTarget,
  selectLogTargetByRecency,
} from "../../src/logging/reader.js";

const TMP_DIR = path.join(process.cwd(), ".tmp-log-reader");

const writeLogFile = (date: string, lines: string[]): string => {
  const filePath = path.join(TMP_DIR, `${date}.jsonl`);
  const content = lines.length ? `${lines.join("\n")}\n` : "";
  fs.writeFileSync(filePath, content);
  return filePath;
};

const sampleLine = (message: string) =>
  JSON.stringify({
    level: 30,
    time: "2026-01-08T00:00:00.000Z",
    category: "cli",
    message,
  });

describe("readLogLinesForDate", () => {
  beforeEach(() => {
    fs.rmSync(TMP_DIR, { recursive: true, force: true });
    fs.mkdirSync(TMP_DIR, { recursive: true });
  });

  afterEach(() => {
    fs.rmSync(TMP_DIR, { recursive: true, force: true });
  });

  it("returns the preferred date file when it has content", async () => {
    const line = sampleLine("preferred");
    writeLogFile("2026-01-08", [line]);

    const result = await readLogLinesForDate(TMP_DIR, "2026-01-08");

    expect(result?.date).toBe("2026-01-08");
    expect(result?.lines).toEqual([line]);
  });

  it("falls back to the latest available log file when preferred is missing", async () => {
    const latest = sampleLine("latest");
    const older = sampleLine("older");
    const latestPath = writeLogFile("2026-01-07", [latest]);
    const olderPath = writeLogFile("2026-01-05", [older]);
    const oldTime = new Date("2026-01-06T00:00:00.000Z");
    const newTime = new Date("2026-01-09T00:00:00.000Z");
    fs.utimesSync(olderPath, oldTime, oldTime);
    fs.utimesSync(latestPath, newTime, newTime);

    const result = await readLogLinesForDate(TMP_DIR, "2026-01-08");

    expect(result?.date).toBe("2026-01-07");
    expect(result?.lines).toEqual([latest]);
  });

  it("falls back to the latest log file with content when preferred is empty", async () => {
    const fallback = sampleLine("fallback");
    writeLogFile("2026-01-08", []);
    writeLogFile("2026-01-07", [fallback]);

    const result = await readLogLinesForDate(TMP_DIR, "2026-01-08");

    expect(result?.date).toBe("2026-01-07");
    expect(result?.lines).toEqual([fallback]);
  });

  it("prefers newest mtime when falling back to available logs", async () => {
    const olderLine = sampleLine("older");
    const newestLine = sampleLine("newest");
    const olderPath = writeLogFile("2026-01-07", [olderLine]);
    const newestPath = writeLogFile("2026-01-05", [newestLine]);
    const oldTime = new Date("2026-01-08T00:00:00.000Z");
    const newTime = new Date("2026-01-10T00:00:00.000Z");
    fs.utimesSync(olderPath, oldTime, oldTime);
    fs.utimesSync(newestPath, newTime, newTime);

    const result = await readLogLinesForDate(TMP_DIR, "2026-01-09");

    expect(result?.date).toBe("2026-01-05");
    expect(result?.lines).toEqual([newestLine]);
  });
});

describe("clearLogFiles", () => {
  beforeEach(() => {
    fs.rmSync(TMP_DIR, { recursive: true, force: true });
    fs.mkdirSync(TMP_DIR, { recursive: true });
  });

  afterEach(() => {
    fs.rmSync(TMP_DIR, { recursive: true, force: true });
  });

  it("clears only log files in the target directory", async () => {
    const line = sampleLine("keep");
    const first = writeLogFile("2026-01-08", [line]);
    const second = writeLogFile("2026-01-07", [line]);
    const other = path.join(TMP_DIR, "notes.txt");
    fs.writeFileSync(other, "do not delete");

    const cleared = await clearLogFiles(TMP_DIR);

    expect(cleared).toBe(2);
    expect(fs.existsSync(first)).toBe(true);
    expect(fs.readFileSync(first, "utf-8")).toBe("");
    expect(fs.existsSync(second)).toBe(true);
    expect(fs.readFileSync(second, "utf-8")).toBe("");
    expect(fs.existsSync(other)).toBe(true);
  });
});

describe("resolveLogTarget", () => {
  it("prefers accessible worktree logs for the selected branch", () => {
    const branch = {
      name: "feature/logs",
      isCurrent: false,
      worktree: { path: "/tmp/feature-logs", isAccessible: true },
    };

    const result = resolveLogTarget(branch, "/repo/root");

    expect(result.logDir).toBe(resolveLogDir("/tmp/feature-logs"));
    expect(result.sourcePath).toBe("/tmp/feature-logs");
    expect(result.reason).toBe("worktree");
  });

  it("falls back to working directory for current branch without worktree", () => {
    const branch = {
      name: "main",
      isCurrent: true,
      worktree: undefined,
    };

    const result = resolveLogTarget(branch, "/repo/root");

    expect(result.logDir).toBe(resolveLogDir("/repo/root"));
    expect(result.sourcePath).toBe("/repo/root");
    expect(result.reason).toBe("current-working-directory");
  });

  it("returns no log dir when worktree is inaccessible", () => {
    const branch = {
      name: "feature/broken",
      isCurrent: false,
      worktree: { path: "/tmp/broken", isAccessible: false },
    };

    const result = resolveLogTarget(branch, "/repo/root");

    expect(result.logDir).toBeNull();
    expect(result.sourcePath).toBe("/tmp/broken");
    expect(result.reason).toBe("worktree-inaccessible");
  });

  it("returns no log dir when branch has no worktree and is not current", () => {
    const branch = {
      name: "feature/no-worktree",
      isCurrent: false,
      worktree: undefined,
    };

    const result = resolveLogTarget(branch, "/repo/root");

    expect(result.logDir).toBeNull();
    expect(result.sourcePath).toBeNull();
    expect(result.reason).toBe("no-worktree");
  });

  it("uses working directory when no branch is provided", () => {
    const result = resolveLogTarget(null, "/repo/root");

    expect(result.logDir).toBe(resolveLogDir("/repo/root"));
    expect(result.sourcePath).toBe("/repo/root");
    expect(result.reason).toBe("working-directory");
  });
});

describe("selectLogTargetByRecency", () => {
  const PRIMARY_DIR = path.join(TMP_DIR, "primary");
  const FALLBACK_DIR = path.join(TMP_DIR, "fallback");

  beforeEach(() => {
    fs.rmSync(TMP_DIR, { recursive: true, force: true });
    fs.mkdirSync(PRIMARY_DIR, { recursive: true });
    fs.mkdirSync(FALLBACK_DIR, { recursive: true });
  });

  afterEach(() => {
    fs.rmSync(TMP_DIR, { recursive: true, force: true });
  });

  it("falls back to working directory when primary logs are older", async () => {
    const primaryLine = sampleLine("primary");
    const fallbackLine = sampleLine("fallback");
    const primaryPath = path.join(PRIMARY_DIR, "2026-01-08.jsonl");
    const fallbackPath = path.join(FALLBACK_DIR, "2026-01-08.jsonl");
    fs.writeFileSync(primaryPath, `${primaryLine}\n`);
    fs.writeFileSync(fallbackPath, `${fallbackLine}\n`);

    const oldTime = new Date("2026-01-08T00:00:00.000Z");
    const newTime = new Date("2026-01-10T00:00:00.000Z");
    fs.utimesSync(primaryPath, oldTime, oldTime);
    fs.utimesSync(fallbackPath, newTime, newTime);

    const primary = {
      logDir: PRIMARY_DIR,
      sourcePath: "/tmp/worktree",
      reason: "worktree" as const,
    };
    const fallback = {
      logDir: FALLBACK_DIR,
      sourcePath: "/repo/root",
      reason: "working-directory" as const,
    };

    const selected = await selectLogTargetByRecency(primary, fallback);

    expect(selected.logDir).toBe(FALLBACK_DIR);
    expect(selected.reason).toBe("working-directory-fallback");
  });
});
