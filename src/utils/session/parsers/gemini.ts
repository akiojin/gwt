/**
 * Gemini CLI session parser
 *
 * Handles session detection for Gemini CLI.
 * Session files are stored in ~/.gemini/tmp/<project_hash>/
 */

import path from "node:path";
import { homedir } from "node:os";

import type { GeminiSessionInfo, SessionSearchOptions } from "../types.js";
import { collectFilesRecursive, readSessionInfoFromFile } from "../common.js";

/**
 * Finds the latest Gemini session with optional time filtering and cwd matching.
 */
export async function findLatestGeminiSession(
  _cwd: string,
  options: SessionSearchOptions = {},
): Promise<GeminiSessionInfo | null> {
  // Gemini stores sessions/logs under ~/.gemini/tmp/<project_hash>/
  const baseDir = path.join(homedir(), ".gemini", "tmp");
  const files = await collectFilesRecursive(
    baseDir,
    (name) => name.endsWith(".json") || name.endsWith(".jsonl"),
  );
  if (!files.length) return null;

  let pool = files;
  const sinceVal = options.since;
  if (sinceVal !== undefined) {
    pool = pool.filter((f) => f.mtime >= sinceVal);
  }
  const untilVal = options.until;
  if (untilVal !== undefined) {
    pool = pool.filter((f) => f.mtime <= untilVal);
  }
  const hasWindow = options.since !== undefined || options.until !== undefined;
  if (!pool.length) {
    if (!hasWindow) {
      pool = files;
    } else {
      return null;
    }
  }

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
      if (
        info.cwd &&
        (info.cwd === options.cwd || info.cwd.startsWith(options.cwd))
      ) {
        return { id: info.id, mtime: file.mtime };
      }
      continue;
    }
    return { id: info.id, mtime: file.mtime };
  }

  return null;
}

/**
 * Finds the latest Gemini session ID.
 */
export async function findLatestGeminiSessionId(
  cwd: string,
  options: SessionSearchOptions = {},
): Promise<string | null> {
  const normalized: Omit<SessionSearchOptions, "cwd"> = {};
  if (options.since !== undefined) normalized.since = options.since;
  if (options.until !== undefined) normalized.until = options.until;
  if (options.preferClosestTo !== undefined)
    normalized.preferClosestTo = options.preferClosestTo;
  if (options.windowMs !== undefined) normalized.windowMs = options.windowMs;

  const found = await findLatestGeminiSession(cwd, {
    ...normalized,
    cwd: options.cwd ?? cwd,
  });
  return found?.id ?? null;
}
