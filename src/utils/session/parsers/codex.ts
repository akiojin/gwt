/**
 * Codex CLI session parser
 *
 * Handles session detection and management for Codex CLI.
 * Session files are stored in ~/.codex/sessions/ (or CODEX_HOME/sessions/).
 * Filename pattern: rollout-YYYY-MM-DDTHH-MM-SS-{uuid}.jsonl
 */

import path from "node:path";
import { homedir } from "node:os";

import type { CodexSessionInfo, SessionSearchOptions } from "../types.js";
import {
  UUID_REGEX,
  collectFilesIterative,
  matchesCwd,
  resolveBranchFromCwd,
  readSessionInfoFromFile,
} from "../common.js";
import { listAllWorktrees } from "../../../worktree.js";

/**
 * Finds the latest Codex session with optional time filtering and cwd matching.
 *
 * Session ID is extracted from:
 * 1. Filename (rollout-...-{uuid}.jsonl) - most reliable
 * 2. File content (payload.id or sessionId fields)
 *
 * @param options - Search options including time filters and cwd matching
 * @returns Session info with ID and modification time, or null if not found
 */
export async function findLatestCodexSession(
  options: SessionSearchOptions = {},
): Promise<CodexSessionInfo | null> {
  // Codex CLI respects CODEX_HOME. Default is ~/.codex.
  const codexHome = process.env.CODEX_HOME ?? path.join(homedir(), ".codex");
  const baseDir = path.join(codexHome, "sessions");
  const candidates = await collectFilesIterative(
    baseDir,
    (name) => name.endsWith(".json") || name.endsWith(".jsonl"),
  );
  if (!candidates.length) return null;

  // Apply time filters
  let pool = candidates;
  const sinceVal = options.since;
  const untilVal = options.until;
  if (sinceVal !== undefined) {
    pool = pool.filter((c) => c.mtime >= sinceVal);
  }
  if (untilVal !== undefined) {
    pool = pool.filter((c) => c.mtime <= untilVal);
  }
  if (!pool.length) return null;

  const ref = options.preferClosestTo;
  const window = options.windowMs ?? 30 * 60 * 1000; // 30 minutes default
  const ordered = [...pool].sort((a, b) => {
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

  for (const file of ordered) {
    // Priority 1: Extract session ID from filename (most reliable for Codex)
    const filenameMatch = path.basename(file.fullPath).match(UUID_REGEX);
    const idFromName = filenameMatch?.[0] ?? null;
    const needsInfo = shouldCheckBranch || shouldCheckCwd || !idFromName;
    const info = needsInfo
      ? await readSessionInfoFromFile(file.fullPath)
      : null;
    const sessionCwd = info?.cwd ?? null;

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

    const sessionId = idFromName ?? info?.id ?? null;
    if (sessionId) {
      return { id: sessionId, mtime: file.mtime };
    }

    // (already handled via info above)
  }

  return null;
}

/**
 * Finds the latest Codex session ID.
 * @param options - Search options including time filters and cwd matching
 * @returns Session ID string or null if not found
 */
export async function findLatestCodexSessionId(
  options: SessionSearchOptions = {},
): Promise<string | null> {
  const found = await findLatestCodexSession(options);
  return found?.id ?? null;
}

/**
 * Polls for a Codex session ID until found or timeout.
 * @param options - Polling options including startedAt time, timeout, and cwd filter
 * @returns Session ID string or null if timeout reached
 */
export async function waitForCodexSessionId(options: {
  startedAt: number;
  timeoutMs?: number;
  pollIntervalMs?: number;
  cwd?: string | null;
}): Promise<string | null> {
  const timeoutMs = options.timeoutMs ?? 120_000;
  const pollIntervalMs = options.pollIntervalMs ?? 2_000;
  const deadline = Date.now() + timeoutMs;

  while (Date.now() < deadline) {
    const found = await findLatestCodexSession({
      since: options.startedAt,
      preferClosestTo: options.startedAt,
      windowMs: 10 * 60 * 1000,
      cwd: options.cwd ?? null,
    });
    if (found?.id) return found.id;
    await new Promise((resolve) => setTimeout(resolve, pollIntervalMs));
  }
  return null;
}
