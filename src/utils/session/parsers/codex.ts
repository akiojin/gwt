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
  readSessionInfoFromFile,
} from "../common.js";

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

  let fallbackMissingCwd: CodexSessionInfo | null = null;

  for (const file of ordered) {
    // Priority 1: Extract session ID from filename (most reliable for Codex)
    const filenameMatch = path.basename(file.fullPath).match(UUID_REGEX);
    if (filenameMatch) {
      const sessionId = filenameMatch[0];
      // If cwd filtering is needed, read file content to check cwd
      if (options.cwd) {
        const info = await readSessionInfoFromFile(file.fullPath);
        if (matchesCwd(info.cwd, options.cwd)) {
          return { id: sessionId, mtime: file.mtime };
        }
        if (!info.cwd && !fallbackMissingCwd) {
          fallbackMissingCwd = { id: sessionId, mtime: file.mtime };
        }
        continue; // cwd doesn't match, try next file
      }
      return { id: sessionId, mtime: file.mtime };
    }

    // Priority 2: Fallback to reading file content if filename lacks UUID
    const info = await readSessionInfoFromFile(file.fullPath);
    if (!info.id) continue;
    if (options.cwd) {
      if (matchesCwd(info.cwd, options.cwd)) {
        return { id: info.id, mtime: file.mtime };
      }
      if (!info.cwd && !fallbackMissingCwd) {
        fallbackMissingCwd = { id: info.id, mtime: file.mtime };
      }
      continue;
    }
    return { id: info.id, mtime: file.mtime };
  }

  return fallbackMissingCwd;
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
