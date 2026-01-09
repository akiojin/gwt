import fs from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import { formatDate } from "./logger.js";

export interface LogFileInfo {
  date: string;
  path: string;
  mtimeMs: number;
}

export type LogTargetReason =
  | "worktree"
  | "worktree-inaccessible"
  | "current-working-directory"
  | "working-directory"
  | "no-worktree";

export interface LogTargetBranch {
  name: string;
  isCurrent?: boolean;
  worktree?: { path: string; isAccessible?: boolean } | undefined;
}

export interface LogTargetResolution {
  logDir: string | null;
  sourcePath: string | null;
  reason: LogTargetReason;
}

const LOG_FILENAME_PATTERN = /^\d{4}-\d{2}-\d{2}\.jsonl$/;

export function resolveLogDir(cwd: string = process.cwd()): string {
  const cwdBase = path.basename(cwd) || "workspace";
  return path.join(os.homedir(), ".gwt", "logs", cwdBase);
}

export function resolveLogTarget(
  branch: LogTargetBranch | null,
  workingDirectory: string = process.cwd(),
): LogTargetResolution {
  if (!branch) {
    return {
      logDir: resolveLogDir(workingDirectory),
      sourcePath: workingDirectory,
      reason: "working-directory",
    };
  }

  const worktreePath = branch.worktree?.path;
  if (worktreePath) {
    const accessible = branch.worktree?.isAccessible !== false;
    if (accessible) {
      return {
        logDir: resolveLogDir(worktreePath),
        sourcePath: worktreePath,
        reason: "worktree",
      };
    }
    return {
      logDir: null,
      sourcePath: worktreePath,
      reason: "worktree-inaccessible",
    };
  }

  if (branch.isCurrent) {
    return {
      logDir: resolveLogDir(workingDirectory),
      sourcePath: workingDirectory,
      reason: "current-working-directory",
    };
  }

  return { logDir: null, sourcePath: null, reason: "no-worktree" };
}

export function buildLogFilePath(logDir: string, date: string): string {
  return path.join(logDir, `${date}.jsonl`);
}

export function getTodayLogDate(): string {
  return formatDate(new Date());
}

export async function readLogFileLines(filePath: string): Promise<string[]> {
  try {
    const content = await fs.readFile(filePath, "utf-8");
    return content.split("\n").filter(Boolean);
  } catch (error) {
    const err = error as NodeJS.ErrnoException;
    if (err.code === "ENOENT") {
      return [];
    }
    throw error;
  }
}

export async function listLogFiles(logDir: string): Promise<LogFileInfo[]> {
  try {
    const entries = await fs.readdir(logDir, { withFileTypes: true });
    const files: LogFileInfo[] = [];

    for (const entry of entries) {
      if (!entry.isFile()) continue;
      if (!LOG_FILENAME_PATTERN.test(entry.name)) continue;

      const date = entry.name.replace(/\.jsonl$/, "");
      const fullPath = path.join(logDir, entry.name);
      try {
        const stat = await fs.stat(fullPath);
        files.push({ date, path: fullPath, mtimeMs: stat.mtimeMs });
      } catch {
        // Ignore stat errors per-file
      }
    }

    return files.sort((a, b) => b.date.localeCompare(a.date));
  } catch (error) {
    const err = error as NodeJS.ErrnoException;
    if (err.code === "ENOENT") {
      return [];
    }
    throw error;
  }
}

export async function clearLogFiles(logDir: string): Promise<number> {
  const files = await listLogFiles(logDir);
  let cleared = 0;
  for (const file of files) {
    try {
      await fs.truncate(file.path, 0);
      cleared += 1;
    } catch (error) {
      const err = error as NodeJS.ErrnoException;
      if (err.code !== "ENOENT") {
        throw error;
      }
    }
  }
  return cleared;
}

export async function listRecentLogFiles(
  logDir: string,
  days = 7,
): Promise<LogFileInfo[]> {
  const files = await listLogFiles(logDir);
  const cutoff = Date.now() - days * 24 * 60 * 60 * 1000;
  return files.filter((file) => file.mtimeMs >= cutoff);
}

export interface LogReadResult {
  date: string;
  lines: string[];
}

export async function readLogLinesForDate(
  logDir: string,
  preferredDate: string,
): Promise<LogReadResult | null> {
  const files = await listLogFiles(logDir);
  if (files.length === 0) {
    return null;
  }

  const ordered: LogFileInfo[] = [];
  const preferred = files.find((file) => file.date === preferredDate);
  if (preferred) {
    ordered.push(preferred);
  }
  for (const file of files) {
    if (preferred && file.date === preferred.date) {
      continue;
    }
    ordered.push(file);
  }

  for (const file of ordered) {
    const lines = await readLogFileLines(file.path);
    if (lines.length > 0) {
      return { date: file.date, lines };
    }
  }

  const fallback = files[0];
  if (!fallback) {
    return { date: preferredDate, lines: [] };
  }
  const fallbackDate = preferred?.date ?? fallback.date;
  return { date: fallbackDate, lines: [] };
}
