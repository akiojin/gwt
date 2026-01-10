/**
 * Gemini CLI session parser
 *
 * Handles session detection for Gemini CLI.
 * Session files are stored in ~/.gemini/tmp/<project_hash>/
 */

import path from "node:path";
import { homedir } from "node:os";

import type { GeminiSessionInfo, SessionSearchOptions } from "../types.js";
import {
  collectFilesIterative,
  matchesCwd,
  resolveBranchFromCwd,
  readSessionInfoFromFile,
} from "../common.js";
import { listAllWorktrees } from "../../../worktree.js";

/**
 * Finds the latest Gemini session with optional time filtering and cwd matching.
 *
 * @param options - Search options including time filters and cwd matching
 * @returns Session info with ID and modification time, or null if not found
 */
export async function findLatestGeminiSession(
  options: SessionSearchOptions = {},
): Promise<GeminiSessionInfo | null> {
  // Gemini stores sessions/logs under ~/.gemini/tmp/<project_hash>/
  const baseDir = path.join(homedir(), ".gemini", "tmp");
  const files = await collectFilesIterative(
    baseDir,
    (name) => name.endsWith(".json") || name.endsWith(".jsonl"),
  );
  if (!files.length) return null;

  // Apply time filters
  let pool = files;
  const sinceVal = options.since;
  if (sinceVal !== undefined) {
    pool = pool.filter((f) => f.mtime >= sinceVal);
  }
  const untilVal = options.until;
  if (untilVal !== undefined) {
    pool = pool.filter((f) => f.mtime <= untilVal);
  }
  if (!pool.length) return null;

  // Sort by preference or mtime
  const ref = options.preferClosestTo;
  const window = options.windowMs ?? 30 * 60 * 1000;
  pool = pool.slice().sort((a, b) => {
    if (typeof ref === "number") {
      const da = Math.abs(a.mtime - ref);
      const db = Math.abs(b.mtime - ref);
      if (da === db) return b.mtime - a.mtime;
      if (da <= window || db <= window) return da - db;
    }
    return b.mtime - a.mtime;
  });

  const branchFilter =
    typeof options.branch === "string" && options.branch.trim().length > 0
      ? options.branch.trim()
      : null;
  const shouldCheckBranch = Boolean(branchFilter);
  const shouldCheckCwd = Boolean(options.cwd) && !shouldCheckBranch;

  let worktrees: { path: string; branch: string }[] = [];
  if (shouldCheckBranch) {
    if (Array.isArray(options.worktrees) && options.worktrees.length > 0) {
      worktrees = options.worktrees
        .filter((entry) => entry?.path && entry?.branch)
        .map((entry) => ({ path: entry.path, branch: entry.branch }));
    } else {
      try {
        const allWorktrees = await listAllWorktrees();
        worktrees = allWorktrees
          .filter((entry) => entry?.path && entry?.branch)
          .map((entry) => ({ path: entry.path, branch: entry.branch }));
      } catch {
        worktrees = [];
      }
    }
    if (!worktrees.length) return null;
  }

  for (const file of pool) {
    const info = await readSessionInfoFromFile(file.fullPath);
    if (!info.id) continue;
    const sessionCwd = info.cwd ?? null;

    if (shouldCheckBranch) {
      const resolvedBranch = resolveBranchFromCwd(sessionCwd, worktrees);
      if (resolvedBranch !== branchFilter) {
        continue;
      }
    }

    if (shouldCheckCwd && options.cwd) {
      if (!matchesCwd(sessionCwd, options.cwd)) {
        continue;
      }
    }

    return { id: info.id, mtime: file.mtime };
  }

  return null;
}

/**
 * Finds the latest Gemini session ID.
 * @param cwd - The working directory to find sessions for (used as fallback if options.cwd not set)
 * @param options - Search options including time filters and cwd matching
 * @returns Session ID string or null if not found
 */
export async function findLatestGeminiSessionId(
  cwd: string,
  options: SessionSearchOptions = {},
): Promise<string | null> {
  const searchOptions: SessionSearchOptions = {
    cwd: options.cwd ?? cwd,
    branch: options.branch ?? null,
    worktrees: options.worktrees ?? null,
  };
  if (options.since !== undefined) searchOptions.since = options.since;
  if (options.until !== undefined) searchOptions.until = options.until;
  if (options.preferClosestTo !== undefined)
    searchOptions.preferClosestTo = options.preferClosestTo;
  if (options.windowMs !== undefined) searchOptions.windowMs = options.windowMs;

  const found = await findLatestGeminiSession(searchOptions);
  return found?.id ?? null;
}
