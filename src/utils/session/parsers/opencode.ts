/**
 * OpenCode session parser
 *
 * Handles session detection for OpenCode.
 * Session files are stored in ~/.local/share/opencode/storage/session/[projectID]/[sessionID].json
 * Session JSON contains: id, projectID, directory (cwd), title, created, updated, etc.
 */

import path from "node:path";
import { homedir } from "node:os";

import type { OpenCodeSessionInfo, SessionSearchOptions } from "../types.js";
import {
  collectFilesIterative,
  matchesCwd,
  readSessionInfoFromFile,
} from "../common.js";

/**
 * Returns the OpenCode session directory
 * OpenCode stores sessions in: ~/.local/share/opencode/storage/session/[projectID]/[sessionID].json
 */
function getOpenCodeSessionDir(): string {
  return path.join(
    homedir(),
    ".local",
    "share",
    "opencode",
    "storage",
    "session",
  );
}

/**
 * Finds the latest OpenCode session with optional time filtering and cwd matching.
 *
 * @param options - Search options including time filters and cwd matching
 * @returns Session info with ID and modification time, or null if not found
 */
export async function findLatestOpenCodeSession(
  options: SessionSearchOptions = {},
): Promise<OpenCodeSessionInfo | null> {
  // OpenCode stores sessions under ~/.local/share/opencode/storage/session/[projectID]/
  const baseDir = getOpenCodeSessionDir();
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

  for (const file of pool) {
    const info = await readSessionInfoFromFile(file.fullPath);
    if (!info.id) continue;
    if (options.cwd) {
      if (matchesCwd(info.cwd, options.cwd)) {
        return { id: info.id, mtime: file.mtime };
      }
      continue;
    }
    return { id: info.id, mtime: file.mtime };
  }

  return null;
}

/**
 * Finds the latest OpenCode session ID.
 * @param cwd - The working directory to find sessions for (used as fallback if options.cwd not set)
 * @param options - Search options including time filters and cwd matching
 * @returns Session ID string or null if not found
 */
export async function findLatestOpenCodeSessionId(
  cwd: string,
  options: SessionSearchOptions = {},
): Promise<string | null> {
  const searchOptions: SessionSearchOptions = { cwd: options.cwd ?? cwd };
  if (options.since !== undefined) searchOptions.since = options.since;
  if (options.until !== undefined) searchOptions.until = options.until;
  if (options.preferClosestTo !== undefined)
    searchOptions.preferClosestTo = options.preferClosestTo;
  if (options.windowMs !== undefined) searchOptions.windowMs = options.windowMs;

  const found = await findLatestOpenCodeSession(searchOptions);
  return found?.id ?? null;
}
